# Documentation Style Guide

Use this guide when reviewing the mdBook content in `training/`.

## Voice and tone

- Use a clear, friendly, instructional voice for learners.
- Prefer active voice and direct instructions.
- Use `we` for guided walkthroughs and `you` when addressing actions the reader must take. Avoid switching voice within the same paragraph.
- Keep sentences concise, but do not remove necessary technical context.
- Prefer consistent phrasing across chapters for repeated actions, prerequisites, and outcomes.

## Formatting

- Use Markdown code spans for code identifiers, crate names, commands, filenames, paths, environment variables, Rust attributes, and Rust types. Acronyms such as BLE, DHCP, or MQTT are prose, not code.
- Do not flag terms inside fenced code blocks, command examples, URLs, or generated output unless the surrounding prose is wrong.
- Use sentence punctuation consistently around lists and link text.
- Use title case for headings, for example `Wi-Fi Provisioning` or `Publishing Data to an MQTT Broker`.

## Terminology

Rules of the form "Use `X`, not `Y`" are checked automatically in prose. Code, URLs, link destinations, and mdBook directives may use the exact spelling of package names, commands, and paths.

- Use `Wi-Fi`, not `wifi`, `WiFi`, or `Wi-fi`, in prose.
- Use `` `no_std` `` for the Rust environment in prose.
- Use `ESP32-C3`, not `esp32c3`, `ESP32C3`, or `esp32-c3`, for the chip name in prose.
- Use `Espressif`, not `espressif`, in prose.
- Use `ESP-IDF`, not `esp-idf`, in prose.
- Use `MQTT`, not `mqtt`, in prose.
- Use `OTA`, not `ota`, in prose.
- Use `SoC`, not `SOC`, in prose.
- Use `eFuse`, not `efuse`, in prose.
- Use `Wokwi`, not `wokwi`, in prose.
- Use `Cargo`, not `cargo`, in prose.
- Use `Rust`, not `rust`, in prose.
