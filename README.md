# Quake3 Log Analyzer

This project serves as a comprehensive solution for the Quake 3 log code challenge, employing a robust and flexible architecture for demonstration purposes. While the complexity might seem overkill for the problem at hand, the project aims to showcase how larger, more complex systems could be designed and implemented.

Although the architecture does introduce some minor performance trade-offs (such as data copying to internal models), these are considered negligible and are outweighed by the architecture's benefits. For performance-critical decisions, refer to the `benches/` directory in key crates.

# Testing & building

Please, use, for testing:
```
cargo test
```

this for docs generation:
```
cargo doc --no-deps --document-private-items
```

and, for building:
```
RUSTFLAGS="-C target-cpu=native" cargo build --release
```


## Multi-tier Architecture

The project adopts a "Multi-tier" architecture, a software architectural model that isolates different components of the application into discrete layers or "tiers". The workspace is organized into multiple crates, each serving a specific role:

- `quake3-server-events`: Handles the parsing of log files to extract game events. This is a generic library that is agnostic to the business rules.
- `model`: Contains the domain-specific entities, including simplified versions of the Quake 3 events and report models.
- `dal-api` / `dal`: The Data Access Layer.
- `bll-api` / `bll`: The Business Logic Layer, responsible for implementing summarization requirements.
- `presentation`: Manages the Presentation layer, handling JSON report generation as specified.
- `app`: The executable crate.
- `commons`: A utility crate containing common code and performance benchmarks, specifically comparing Iterators and Streams.

This layered approach offers several advantages:

- **Code Decoupling**: Each layer is isolated, ensuring that it neither needs nor can interfere with other layers. This enhances maintainability.
- **Separation of Concerns**: Facilitates parallel development efforts across a team.
- **Testability**: The isolated nature of the layers makes it easier to create mock objects for testing various scenarios.

## Clean Architecture & Dependency Inversion

This project also embraces the principles of Clean Architecture and Dependency Inversion. According to these principles, dependencies should only point inward towards the core entities. This ensures that the outer layers (such as Presentation) depend on the inner layers (like Business Logic), but not vice versa. Importantly, these dependencies are abstract, not concrete.

To support this, the project includes `*-api` crates containing traits, configurations, and other abstract definitions, separate from their concrete implementations.


## Development Process

The development of this project began with a thorough analysis of the provided Quake 3 log file to gain a deeper understanding of the problem at hand. For details, refer to `quake3-server-events/README.md`. This allowed me to build and test a parser in isolation and benchmark different implementations: one based on Regular Expressions and another on the `std::str::*` API.

The `model` crate was then developed to include both simplified Quake 3 events and the report data, which are central to the problem domain. The Quake3 events remodeling was deliberate, as it removes the dependency from the external library from our code and allows the logic layer to be simpler. Please note that the parser was deliberately kept with a greater-than-necessary range of events it can understand to emphasize the benefits of this decision.

Next, I focused on the Data Access Layer (DAL). I opted for a strategy that would accommodate log files of any size. Thus, I used `Stream`s to process data as it becomes available, freeing up memory that would otherwise be used to hold all the data at once. Benchmarks comparing `Stream`s vs `Iterator`s can be found in the `commons` crate.

The Business Logic Layer (BLL) was designed to be short, expressive, decoupled, and modular. This approach accommodates the ever-changing nature of business logic. Various functionalities, including those for the "plus" challenge, were added for validation.

When we reached the Presentation layer, our architecture allowed us to focus solely on JSON report generation. The code was written manually for flexibility and performance, especially for enabling or disabling extra business rules.

Finally, the `app` crate was created to integrate all dependencies into a single executable. For usage instructions and other details, refer to `app/README.md`. A benchmark revealed that the app can process 3 million log lines per second (when built with the recommended command).

## Implementation Notes

The codebase makes extensive use of functional programming paradigms and higher-order functions, not only for `Stream`s but also for `Option`s, `Result`s, and `bool`. Familiarity with these APIs will be beneficial for understanding the code. Despite the initial learning curve, mastering these APIs leads to cleaner, more expressive, and standardized code.

Several design patterns, such as the "Factory Pattern" in the DAL, are employed. Internal patterns, like configuration handling for each tier, are also consistent across the codebase, although not explicitly documented.

You'll also notice that error handling was a significant focus throughout the code. The error messages are designed to provide meaningful insights into the issue at hand, all in a single message, eliminating the need for stack traces. The errors adhere to a specific format: `<module>: Error message: <root_error>`. This allows for nested errors that paint a clear picture of the issue, from the immediate consequences to the root cause.

Last but not least, automated testing is not only heavily used, but it guided the development: the resulting decoupled code is a direct consequence of "writing the test first, coding second". I am confident the implementation is robust, on all layers, due to the thorough testing that was done. You will also see that not only "isolated unit tests" were built, but also "integrated unit tests". The former uses "mock objects" for the dependencies, while the later uses the real implementation -- testing the unit in integration with some of its concrete dependencies, for extra assurance that the parts fit well together.


## Security Concerns

Thanks to Rust's memory safety features, the risk of vulnerabilities such as buffer overflow attacks is minimized. Additionally, the use of `Stream`s ensures that large log files won't exhaust system memory. The remaining concern, not addressed in this version, is related to the potential for large numbers of players and long nicknames to exhaust RAM due to hash table storage. Implementing limits on these aspects could be easily done in both the BLL and the log parser.


## Performance Concerns

The current version of the project is single-threaded and synchronous. Although asynchronous programming is enabled through the use of `Streams`, it has not yet been implemented. Future work could explore the benefits of multi-threading, either synchronous or asynchronous (using Tokio), to improve processing speed.

Based on a thought experiment, the synchronization overhead could negate the benefits of multi-threading. Specifically, distributing events across multiple worker threads would require subsequent synchronization to combine the results for final output -- in addition to the internal synchronizations the channels involved require.

To maximize single-thread performance, several optimizations were made. Most notably, the log file parser was carefully designed to be efficient. Benchmark tests revealed that string manipulation techniques were 3200 times faster than using Regular Expressions for this specific task.

Additionally, custom linker settings were applied (as detailed in the root `Cargo.toml`) to adhere to best practices for generating highly optimized binaries. However, I avoided micro-optimizations that did not result from code cleaning efforts, in order to prioritize code maintainability.


## Cross compilation

For cross compilation to ARM devices, consider installing these packages (listed in ArchLinux's nomenclature):
  - `arm-none-eabi-newlib` & `arm-none-eabi-binutils`: base packages for cross compilation
  - `arm-linux-gnueabihf-gcc`: creates binaries to run on an ARM/Linux system -- such as Raspberry Pi
  - `arm-none-eabi-gcc`: creates binaries to run on a bare metal ARM system
  - `aarch64-linux-gnu-gcc` & `aarch64-linux-gnu-binutils`: support for ARM 64 bits

### Raspberry Pi 1
You need to install the appropriate target:
```bash
rustup target add arm-unknown-linux-gnueabihf
```
Then run the cross-build:
```bash
CC=arm-linux-gnueabihf-gcc CARGO_TARGET_ARM_UNKNOWN_LINUX_GNUEABIHF_LINKER=arm-linux-gnueabihf-gcc cargo build --target=arm-unknown-linux-gnueabihf --release
```

### Raspberry Pi 2
Install the specific target for the machine's CPU:
```bash
rustup target add armv7-unknown-linux-gnueabihf
```
Then build it:
```bash
CC=arm-linux-gnueabihf-gcc CARGO_TARGET_ARMV7_UNKNOWN_LINUX_GNUEABIHF_LINKER=arm-linux-gnueabihf-gcc cargo build --target=armv7-unknown-linux-gnueabihf --release
```

### Raspberry Pi 3, 4 & 5 (64 bits)
Local toolchain:
```bash
rustup target add aarch64-unknown-linux-gnu
```
Cross-compilation:
```bash
CC=aarch64-linux-gnu-gcc CARGO_TARGET_AARCH64_UNKNOWN_LINUX_GNU_LINKER=aarch64-linux-gnu-gcc cargo build --target=aarch64-unknown-linux-gnu --release
```
