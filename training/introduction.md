<div style="text-align:center; margin-top:1.5rem">
  <img src="./_assets/esp-logo-black.svg" width="50%">
</div>

# Introduction

The goal of this book is to provide a detailed getting-started guide for using the Rust programming language with the ESP32 series of devices from Espressif.

This book will guide users through the process of creating a `no_std` Rust application using [esp-hal], demonstrating the use of various hardware peripherals as we progress. More information regarding the application we will be building can be found in the [Project Overview] chapter.

We assume some familiarity with Rust, with more details in the [Prerequisites] chapter.

You can join the [esp-rs community] on Matrix for any technical questions or issues you may have. The community is open to everybody.

[esp-hal]: https://github.com/esp-rs/esp-hal
[esp-hal/examples]: https://github.com/esp-rs/esp-hal/tree/esp-hal-v1.0.0-rc.1/examples
[project overview]: ./application/project-overview.md
[prerequisites]: ./prerequisites.md
[esp-rs community]: https://matrix.to/#/#esp-rs:matrix.org

## How to Use This Book

This book assumes that you're reading it front-to-back. Later chapters build on concepts introduced in earlier chapters, and earlier chapters may omit details which are covered in more depth later on in the book.

Source code for the application we will be building can be found in the [esp-rs/no-std_training] repository.

[esp-rs/no-std_training]: https://github.com/esp-rs/no_std-training/tree/main

## Conventions Used in This Book

We use icons to mark different kinds of information in the book:

- ✅ Call for action.
- ⚠️ Warnings, details that require special attention.
- 🔎 Knowledge that dives deeper into a subject but which you are not required to understand, proceeding.
- 💡 Hints that might help you during the exercises

> Example note: Notes like this one contain helpful information

In some Rust files, you may find some anchor comments:

```rust,ignore
// ANCHOR: test
let foo = 1;
...
// ANCHOR_END: test
```

Anchor comments can be ignored, they are only used to introduce those parts of code in this book. See the [`mdBook` documentation].

[`mdbook` documentation]: https://rust-lang.github.io/mdBook/format/mdbook.html#including-portions-of-a-file

## Re-using This Material

This book is distributed under the following licenses:

- The code contained within this book are licensed under the terms of both the [MIT License] and the [Apache License v2.0].
- The written prose, pictures and diagrams contained within this book are licensed under the terms of the Creative Commons [CC-BY-SA v4.0] license.

If you want to use our text or images in your work, you must:

- Give the appropriate credit (i.e. mention this book on your slide, and provide a link to the relevant page)
- Provide a link to the [CC-BY-SA v4.0] licence
- Indicate if you have changed the material in any way, and make any changes to our material available under the same licence

[MIT License]: https://opensource.org/licenses/MIT
[Apache License v2.0]: http://www.apache.org/licenses/LICENSE-2.0
[CC-BY-SA v4.0]: https://creativecommons.org/licenses/by-sa/4.0/legalcode
