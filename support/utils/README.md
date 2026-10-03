# Playdate Build Utils

Mainly there are utils to find existing (means already installed) Playdate SDK and GNU-ARM toolchain.

Compiler discovery tries `ARM_GCC_PATH`, then `PATH`, then platform-specific locations.
On macOS, the fallback searches `/usr/local/bin`, `/usr/bin`, `/usr/local/playdate/*/bin`, `/Applications/ArmGNUToolchain/*/arm-none-eabi/bin`, and `/opt/homebrew/bin`, in that order.
Versioned installation directories are searched in lexical path order; the first compiler whose `--version` succeeds is used, not necessarily the newest version.
Unusable candidates are skipped, so an incompatible SDK-installed compiler does not prevent selecting a runnable native installation.

Also there are some constants such as hardcoded compilation flags and linker-script.





- - -

This software is not sponsored or supported by Panic.
