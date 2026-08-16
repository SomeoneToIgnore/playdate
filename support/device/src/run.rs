use std::borrow::Cow;
use std::path::PathBuf;

use futures::stream::FuturesUnordered;
use futures::{FutureExt, TryStreamExt};
use futures_lite::StreamExt;

use crate::device::query::Query;
use crate::device::wait_mode_data;
use crate::error::Error;
use crate::mount::UnmountAsync;
use crate::{install, device, usb, interface};


type Result<T = (), E = Error> = std::result::Result<T, E>;


#[cfg_attr(feature = "tracing", tracing::instrument)]
pub async fn run(query: Query,
                 pdx: PathBuf,
                 no_install: bool,
                 no_read: bool,
                 force: bool)
                 -> Result<Vec<device::Device>> {
	use crate::retry::{DefaultIterTime, Retries};
	let wait_data = {
		let mut retry = Retries::<DefaultIterTime>::default();
		retry.total = std::time::Duration::from_secs(60);
		retry
	};


	let to_run: Vec<(device::Device, Cow<str>)> = if !no_install {
		let results = install::mount_and_install(query, &pdx, force).await?
		                                                            .flat_map(|res| {
			                                                            let wait_data = wait_data.clone();
			                                                            async move {
				                                                            let path = res?;
				                                                            let (mount, path) = path.into_parts();
				                                                            mount.unmount().await?;
				                                                            let dev =
					                                                            wait_mode_data(mount.device, wait_data).await?;
				                                                            Ok::<_, Error>((dev, Cow::from(path)))
			                                                            }.into_stream()
		                                                            })
		                                                            .collect::<Vec<Result<(device::Device, Cow<str>)>>>()
		                                                            .await;
		let mut to_run = Vec::with_capacity(results.len());
		let mut failed = None;
		for res in results {
			match res {
				Ok(pair) => to_run.push(pair),
				Err(err) => {
					error!("{err}");
					failed = Some(err);
				},
			}
		}
		if to_run.is_empty() {
			return Err(failed.unwrap_or_else(Error::not_found));
		}
		to_run
	} else {
		usb::discover::devices_data()?.map(|dev| (dev, pdx.to_string_lossy()))
		                              .collect()
	};


	let mut to_read = Vec::with_capacity(to_run.len());
	let readers = FuturesUnordered::new();

	for (mut device, path) in to_run {
		use interface::r#async::Out;

		device.open()?;
		{
			let interface = device.interface()?;
			interface.send_cmd(device::command::Command::Run { path: path.into_owned() })
			         .await?;
		}

		if !no_read {
			to_read.push(device);
		}
	}

	if !no_read {
		for device in to_read.iter_mut() {
			readers.push(usb::io::redirect_to_stdout(device));
		}
	}

	readers.inspect_err(|err| error!("{err}"))
	       .try_for_each_concurrent(8, |_| async { Ok(()) })
	       .await?;

	Ok(to_read)
}
