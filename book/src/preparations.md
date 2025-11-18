# Preparations

This chapter contains information about the course material, the required hardware, and an installation guide.

## Icons and Formatting We Use

We use Icons to mark different kinds of information in the book:
* ✅ Call for action.
* ⚠️ Warnings, details that require special attention.
* 🔎 Knowledge that dives deeper into a subject but which you are not required to understand, proceeding.
* 💡 Hints that might help you during the exercises

> Example note: Notes like this one contain helpful information

## Code Annotations

In some Rust files, you can find some anchor comments:
```rust,ignore
// ANCHOR: test
let foo = 1;
...
// ANCHOR_END: test
```
Anchor comments can be ignored, they are only used to introduce those parts of code in this book. See [`mdBook` documentation](https://rust-lang.github.io/mdBook/format/mdbook.html#including-portions-of-a-file)

## Required Hardware

- [Rust ESP Board](https://github.com/esp-rs/esp-rust-board): available on Mouser, Aliexpress. [Full list of vendors](https://github.com/esp-rs/esp-rust-board#where-to-buy).
- USB-C cable suitable to connect the board to your development computer.
- Wi-Fi access point connected to the Internet.

> No additional debugger/probe hardware is required.

## Simulating Projects

Certain projects can be simulated with [Wokwi][wokwi]. Look for indications in the book to identify projects available for simulation. Simulation can be accomplished through two methods:
- Using wokwi.com: Conduct the build process and code editing directly through the browser.
- Using [Wokwi VS Code extension][wokwi-vscode]: Leverage VS Code to edit projects and perform builds. Utilize the Wokwi VS Code extension to simulate the resulting binaries.
    - This approach requires some [installation][wokwi-installation]
    - This approach assumes that the project is built in debug mode
    - This approach allows [debugging the project][wokwi-debug]

[wokwi]: https://wokwi.com/
[wokwi-vscode]: https://docs.wokwi.com/vscode/getting-started
[wokwi-installation]: https://docs.wokwi.com/vscode/getting-started#installation
[wokwi-debug]: https://docs.wokwi.com/vscode/debugging

## Companion Material

- [Official esp-rs book](https://esp-rs.github.io/book/introduction.html)
