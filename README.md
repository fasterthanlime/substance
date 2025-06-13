# substance

A Rust library for analyzing the size composition of binaries by examining their symbols and mapping them back to their originating crates.

Supports ELF (Linux, BSD), Mach-O (macOS) and PE (Windows) binaries. Originally derived from cargo-bloat but redesigned as a library.

## Attribution

- **Binary analysis**: Originally derived from [cargo-bloat](https://github.com/RazrFalcon/cargo-bloat) by RazrFalcon
- **LLVM IR analysis**: Inspired by [cargo-llvm-lines](https://github.com/dtolnay/cargo-llvm-lines) by dtolnay, which was originally suggested by @eddyb for debugging compiler memory usage and compile times

## License

Licensed under the MIT license.
