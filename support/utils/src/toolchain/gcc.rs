//! ARM GNU toolchain

use std::borrow::Cow;
use std::ffi::OsStr;
#[cfg(target_os = "macos")]
use std::fs;
use std::io::ErrorKind;
use std::path::Path;
use std::path::PathBuf;
use std::process::Command;
use std::io::Error as IoError;
use std::process::Stdio;

use self::err::Error;


/// Env var name that points to the arm-gcc executable.
pub const ARM_GCC_PATH_ENV_VAR: &str = "ARM_GCC_PATH";

/// Variants of the compile's name - actual and old.
pub const ARM_NONE_EABI_GCC: &[&str] = &["arm-none-eabi-gcc", "gcc-arm-none-eabi"];


pub struct Gcc {
	path: PathBuf,
}

pub struct ArmToolchain {
	gcc: Gcc,
	sysroot: PathBuf,
}


impl Gcc {
	pub fn path(&self) -> &Path { self.path.as_path() }


	/// Automatically determine the gcc
	pub fn try_new() -> Result<Self, Error> {
		let try_with = |f: fn() -> Result<Self, Error>| {
			move |err: Error| {
				let result = f();
				if result.is_err() {
					crate::debug!("{err}");
				}
				result
			}
		};

		Self::try_from_default_env().or_else(try_with(Self::try_from_env_path))
		                            .or_else(try_with(Self::try_from_default_path))
	}


	pub fn try_new_exact_path<P: Into<PathBuf>>(path: P) -> Result<Self, Error> {
		let path = path.into().canonicalize()?;
		if path.try_exists()? {
			Ok(Self { path })
		} else {
			Err(IoError::new(
				ErrorKind::NotFound,
				format!("Could not find ARM GCC at '{}'", path.display()),
			).into())
		}
	}


	/// Create new with default env var
	pub fn try_from_default_env() -> Result<Self, Error> {
		let res = std::env::var_os(ARM_GCC_PATH_ENV_VAR).map(PathBuf::from)
		                                                .map(Self::try_new_exact_path);
		res.ok_or(IoError::new(ErrorKind::NotFound, format!("Missed env {ARM_GCC_PATH_ENV_VAR}")))?
	}

	/// Create new with executable in PATH
	pub fn try_from_env_path() -> Result<Self, Error> {
		for name in ARM_NONE_EABI_GCC {
			if let Ok(result) = Self::try_from_path(name) {
				return Ok(result);
			}
		}

		Err(Error::Err("Could not find ARM GCC in PATH"))
	}

	/// Create new with executable name or path
	pub fn try_from_path<S: AsRef<OsStr>>(path: S) -> Result<Self, Error> {
		let mut proc = Command::new(path.as_ref());
		proc.arg("--version");
		let output = proc.output()?;
		if !output.status.success() {
			return Err(Error::exit_status_error(&proc, output.stderr, output.status));
		}
		Ok(Self { path: path.as_ref().into() })
	}

	/// Create new with default path of executable
	pub fn try_from_default_path() -> Result<Self, Error> {
		#[cfg(unix)]
		{
			Self::try_from_default_paths(Path::new("/"))
		}

		#[cfg(windows)]
		{
			let path =
				PathBuf::from(r"C:\Program Files (x86)\GNU Tools Arm Embedded\9 2019-q4-major\bin\").join(ARM_NONE_EABI_GCC[0])
				                                                                                    .with_extension("exe");
			Self::try_from_path(path).map_err(|_| Error::Err("Could not find ARM toolchain in default paths"))
		}
	}


	/// Determine sysroot.
	// There're another ways to do this.
	// For example we can parse makefile in PlaydateSDK to get the path, but that's ugly way.
	fn sysroot(&self) -> Result<PathBuf, Error> { self.sysroot_by_output().or_else(|_| self.sysroot_fallback()) }

	/// Determine by asking gcc.
	fn sysroot_by_output(&self) -> Result<PathBuf, Error> {
		let mut proc = Command::new(&self.path);
		proc.arg("-print-sysroot");

		let output = proc.output()?;
		if !output.status.success() {
			return Err(Error::exit_status_error(&proc, output.stderr, output.status));
		}
		let path = std::str::from_utf8(&output.stdout).map(str::trim)
		                                              .map(PathBuf::from)?;

		if path.as_os_str().is_empty() {
			Err(Error::Err("gcc returns empty string for sysroot"))
		} else {
			Ok(path.canonicalize()?)
		}
	}

	/// Determine by path, relative to the gcc path.
	fn sysroot_fallback(&self) -> Result<PathBuf, Error> {
		// just name in PATH | full path
		let path = if self.path.is_relative() || self.path.components().count() == 1 {
			           let mut proc = Command::new("which");
			           proc.arg(&self.path);
			           let output = proc.output()?;
			           if !output.status.success() {
				           return Err(Error::exit_status_error(&proc, output.stderr, output.status));
			           }
			           crate::debug!("path by which: {:?}", std::str::from_utf8(&output.stdout));
			           Cow::from(std::str::from_utf8(&output.stdout).map(str::trim)
			                                                        .map(PathBuf::from)?)
		           } else {
			           Cow::from(self.path.as_path())
		           }.canonicalize()?;


		let path = path.parent()
		               .and_then(|p| p.parent())
		               .map(|p| p.join("arm-none-eabi"))
		               .ok_or(IoError::new(ErrorKind::NotFound, "GCC sysroot not found"))?;

		if !path.exists() && path == PathBuf::from("/usr/arm-none-eabi") {
			let path = PathBuf::from("/usr/lib/arm-none-eabi");
			if path.exists() {
				return Ok(path);
			}
		}

		crate::trace!("trying canonicalize this: {}", path.display());
		let path = path.canonicalize()?;
		Ok(path)
	}

	#[cfg(unix)]
	fn try_from_default_paths(root: &Path) -> Result<Self, Error> {
		let directories = [root.join("usr/local/bin"), root.join("usr/bin")].into_iter();
		#[cfg(target_os = "macos")]
		let directories = {
			let mut additional = Vec::new();
			for (directory, bin) in [
			                         ("usr/local/playdate", "bin"),
			                         ("Applications/ArmGNUToolchain", "arm-none-eabi/bin"),
			] {
				let mut installations = fs::read_dir(root.join(directory)).into_iter()
				                                                          .flatten()
				                                                          .filter_map(Result::ok)
				                                                          .map(|entry| entry.path().join(bin))
				                                                          .collect::<Vec<_>>();
				installations.sort_unstable();
				additional.extend(installations);
			}
			additional.push(root.join("opt/homebrew/bin"));
			directories.chain(additional)
		};
		let paths =
			directories.flat_map(|directory| ARM_NONE_EABI_GCC.iter().map(move |name| directory.join(name)))
			           .filter(|path| path.try_exists().ok().unwrap_or_default());
		for path in paths {
			match Self::try_from_path(&path) {
				Ok(gcc) => return Ok(gcc),
				Err(err) => crate::debug!("{}: {err:?}", path.display()),
			}
		}
		Err(Error::Err("Could not find ARM toolchain in default paths"))
	}
}


impl ArmToolchain {
	pub fn gcc(&self) -> &Gcc { &self.gcc }
	pub fn bin(&self) -> PathBuf { self.sysroot.join("bin") }
	pub fn lib(&self) -> PathBuf { self.sysroot.join("lib") }
	pub fn include(&self) -> PathBuf { self.sysroot.join("include") }
	pub fn sysroot(&self) -> &Path { self.sysroot.as_ref() }


	/// Specialized search-path for target
	// e.g.: arm-none-eabi-gcc -mthumb -mcpu=cortex-m7 -mfloat-abi=hard -mfpu=fpv5-sp-d16 -print-search-dirs
	pub fn lib_search_paths_for<S: AsRef<OsStr>, I: IntoIterator<Item = S>>(&self,
	                                                                        args: I)
	                                                                        -> Result<Vec<PathBuf>, Error> {
		let mut proc = Command::new(self.gcc().path());
		proc.args(args);
		proc.arg("-print-search-dirs");
		proc.stderr(Stdio::inherit());
		proc.stdout(Stdio::piped());

		let output = proc.output()?;
		if !output.status.success() {
			return Err(Error::exit_status_error(&proc, output.stderr, output.status));
		}

		#[cfg(not(windows))]
		const SEP: &str = ":";
		#[cfg(windows)]
		const SEP: &str = ";";

		Ok(std::str::from_utf8(&output.stdout)?.lines()
		                                       .filter_map(|s| s.strip_prefix("libraries: ="))
		                                       .flat_map(|s| s.split(SEP).map(|s| s.trim()).map(PathBuf::from))
		                                       .collect())
	}

	pub fn lib_search_paths_for_playdate(&self) -> Result<Vec<PathBuf>, Error> {
		self.lib_search_paths_for([
			"-mthumb",
			"-mcpu=cortex-m7",
			"-mfloat-abi=hard",
			"-mfpu=fpv5-sp-d16",
		])
	}

	pub fn lib_search_paths_default(&self) -> Result<Vec<PathBuf>, Error> {
		match self.lib_search_paths_for::<&str, _>([]) {
			Ok(paths) if !paths.is_empty() => Ok(paths),
			Ok(_) | Err(_) => Ok(vec![self.gcc().sysroot().map(|p| p.join("lib"))?]),
		}
	}


	/// Create auto-determine the toolchain
	pub fn try_new() -> Result<Self, Error> { Self::try_new_with(Gcc::try_new()?) }

	/// Create auto-determine the toolchain by specified gcc
	pub fn try_new_with(gcc: Gcc) -> Result<Self, Error> {
		let sysroot = gcc.sysroot()?;
		let bin = sysroot.join("bin");
		let lib = sysroot.join("lib");
		let include = sysroot.join("include");

		if !bin.try_exists()? || !lib.try_exists()? || !include.try_exists()? {
			Err(IoError::new(
				ErrorKind::NotFound,
				format!("ARM toolchain not found in '{}'", sysroot.display()),
			).into())
		} else {
			Ok(Self { gcc, sysroot })
		}
	}
}


pub mod err {
	use std::io::Error as IoError;
	use std::process::Command;
	use std::process::ExitStatus;
	use std::str::Utf8Error;

	#[derive(Debug)]
	pub enum Error {
		Io(IoError),
		Utf8(Utf8Error),
		Err(&'static str),
		ExitStatusError {
			cmd: String,
			stderr: Vec<u8>,
			status: ExitStatus,
		},
		// TODO: from `std::process::ExitStatusError` when stabilized `exit_status_error`
	}

	impl From<&'static str> for Error {
		fn from(s: &'static str) -> Self { Self::Err(s) }
	}

	impl From<IoError> for Error {
		fn from(err: IoError) -> Self { Self::Io(err) }
	}
	impl From<Utf8Error> for Error {
		fn from(err: Utf8Error) -> Self { Self::Utf8(err) }
	}

	impl Error {
		pub fn exit_status_error(cmd: &Command, stderr: Vec<u8>, status: ExitStatus) -> Self {
			let cmd = format!(
			                  "{} {}",
			                  cmd.get_program().to_string_lossy(),
			                  cmd.get_args()
			                     .map(|s| s.to_string_lossy())
			                     .collect::<Vec<_>>()
			                     .join(" ")
			);
			Self::ExitStatusError { cmd, stderr, status }
		}
	}

	impl std::error::Error for Error {
		fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
			match self {
				Error::Io(err) => Some(err),
				Error::Utf8(err) => Some(err),
				Error::Err(_) => None,
				Error::ExitStatusError { .. } => None,
			}
		}
	}

	impl std::fmt::Display for Error {
		fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
			match self {
				Error::Io(err) => err.fmt(f),
				Error::Utf8(err) => err.fmt(f),
				Error::Err(err) => err.fmt(f),
				Error::ExitStatusError { cmd, status, stderr } => {
					let stderr = std::str::from_utf8(stderr).map(str::trim)
					                                        .map(|s| format!("with output: {s}"))
					                                        .ok()
					                                        .unwrap_or_else(|| {
						                                        if stderr.is_empty() {
							                                        "without output".into()
						                                        } else {
							                                        "with not decodable output".into()
						                                        }
					                                        });
					write!(f, "ExitStatusError: ({status}) {cmd} {stderr}.",)
				},
			}
		}
	}
}


// TODO: Maybe move this tests to integration tests dir and run if arm-gcc exists only.
#[cfg(test)]
mod tests {
	use super::*;
	#[cfg(target_os = "macos")]
	use std::os::unix::fs::symlink;


	#[test]
	fn gcc_from_env_path() { Gcc::try_from_env_path().unwrap(); }

	#[test]
	#[cfg(unix)]
	fn gcc_from_default_path() { Gcc::try_from_default_path().unwrap(); }


	#[test]
	#[cfg(unix)]
	fn gcc_sysroot_fallback() {
		let gcc = Gcc::try_new().unwrap();
		let res = gcc.sysroot_fallback().unwrap();
		assert!(res.exists());
	}

	#[test]
	#[ignore = "sysroot can be empty"]
	fn gcc_sysroot_by_output() {
		let gcc = Gcc::try_new().unwrap();
		let res = gcc.sysroot_by_output().unwrap();
		assert!(res.exists());
	}


	#[test]
	fn toolchain_new() {
		let toolchain = ArmToolchain::try_new().unwrap();
		assert!(toolchain.bin().exists());
		assert!(toolchain.lib().exists());
		assert!(toolchain.include().exists());
	}


	#[test]
	#[cfg(target_os = "macos")]
	fn gcc_from_macos_installations() {
		for (index, directory) in [
		                           "usr/local/playdate/sdk version/bin",
		                           "Applications/ArmGNUToolchain/native version/arm-none-eabi/bin",
		                           "opt/homebrew/bin",
		].into_iter()
		                          .enumerate()
		{
			let installation = GccInstallation::new(&format!("layout-{index}"));
			let compiler = installation.compiler(directory, ARM_NONE_EABI_GCC[index % 2], "/usr/bin/true");
			assert_eq!(
			           Gcc::try_from_default_paths(&installation.root).unwrap().path(),
			           compiler
			);
		}
	}

	#[test]
	#[cfg(target_os = "macos")]
	fn gcc_from_macos_installations_skips_unusable_candidates() {
		let installation = GccInstallation::new("fallback");
		let name = ARM_NONE_EABI_GCC[0];
		installation.compiler("usr/local/bin", name, "/usr/bin/false");
		installation.compiler("usr/local/playdate/a old/bin", name, "/dev/null");
		let later = installation.compiler("usr/local/playdate/c sdk/bin", name, "/usr/bin/true");
		let sdk = installation.compiler("usr/local/playdate/b sdk/bin", name, "/usr/bin/true");
		let arm = installation.compiler(
		                                "Applications/ArmGNUToolchain/native/arm-none-eabi/bin",
		                                name,
		                                "/usr/bin/true",
		);
		let homebrew = installation.compiler("opt/homebrew/bin", name, "/usr/bin/true");
		for expected in [sdk, later, arm, homebrew] {
			assert_eq!(
			           Gcc::try_from_default_paths(&installation.root).unwrap().path(),
			           expected
			);
			fs::remove_file(expected).unwrap();
		}
		assert_eq!(
		           Gcc::try_from_default_paths(&installation.root).err()
		                                                          .unwrap()
		                                                          .to_string(),
		           "Could not find ARM toolchain in default paths"
		);
	}

	#[cfg(target_os = "macos")]
	struct GccInstallation {
		root: PathBuf,
	}

	#[cfg(target_os = "macos")]
	impl GccInstallation {
		fn new(name: &str) -> Self {
			let root = std::env::temp_dir().join(format!("playdate gcc {name} {}", std::process::id()));
			fs::create_dir(&root).unwrap();
			Self { root }
		}

		fn compiler(&self, directory: &str, name: &str, executable: &str) -> PathBuf {
			let path = self.root.join(directory).join(name);
			fs::create_dir_all(path.parent().unwrap()).unwrap();
			symlink(executable, &path).unwrap();
			path
		}
	}

	#[cfg(target_os = "macos")]
	impl Drop for GccInstallation {
		fn drop(&mut self) { let _ = fs::remove_dir_all(&self.root); }
	}
}
