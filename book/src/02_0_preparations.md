# Preparations

This chapter contains information about the course material, the required hardware, and an installation guide.

## Icons and Formatting we use

We use Icons to mark different kinds of information in the book:
* ✅ Call for action.
* ⚠️ Warnings, details that require special attention.
* 🔎 Knowledge that dives deeper into a subject but which you are not required to understand, proceeding.
* 💡 Hints that might help you during the exercises

> Example note: Notes like this one contain helpful information

## Code annotations

In some Rust files, you can find some anchor comments:
```rust,ignore
// ANCHOR: test
let foo = 1;
...
// ANCHOR_END: test
```
Anchor comments can be ingored, they are only used to introduce those parts of code in this book. See [`mdBook` documentation](https://rust-lang.github.io/mdBook/format/mdbook.html#including-portions-of-a-file)

## Required Hardware

- [Rust ESP Board](https://github.com/esp-rs/esp-rust-board): available on Mouser, Aliexpress. [Full list of vendors](https://github.com/esp-rs/esp-rust-board#where-to-buy).
- USB-C cable suitable to connect the board to your development computer.
- Wi-Fi access point connected to the Internet.

> No additional debugger/probe hardware is required.

## Companion material

- [Official esp-rs book](https://esp-rs.github.io/book/introduction.html)
