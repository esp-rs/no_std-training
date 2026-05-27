# Documentation Style Guide

Use this guide when reviewing the mdBook content in `training/`.

## Voice and tone

- Use a clear, friendly, instructional voice for learners.
- Prefer active voice and direct instructions.
- Use `we` for guided walkthroughs and `you` when addressing actions the reader must take. Avoid switching voice within the same paragraph.
- Keep sentences concise, but do not remove necessary technical context.
- Prefer consistent phrasing across chapters for repeated actions, prerequisites, and outcomes.

## Formatting

- Use Markdown code spans for code identifiers, crate names, commands, filenames, paths, environment variables, Rust attributes, and Rust types.
- Do not flag terms inside fenced code blocks, command examples, URLs, or generated output unless the surrounding prose is wrong.
- Use sentence punctuation consistently around lists and link text.
- Use heading-style capitalization for headings.

## Terminology

- Use `Wi-Fi`, not `wifi`, `WiFi`, or `Wi-fi`, in prose.
- Use `` `no_std` `` for the Rust environment in prose. Repository names, URLs, and commands may use their exact spelling.
- Use `ESP32-C3` for the chip name.
- Use `Espressif`, not `espressif`, in prose.
- Use `ESP-IDF`, not `esp-idf`, in prose. Package names, links, commands, and paths may use exact lowercase spelling.
- Use `MQTT` and `OTA` as acronyms in prose.
- Use `SoC`, not `SOC`.
- Use `eFuse`, not `efuse`.
