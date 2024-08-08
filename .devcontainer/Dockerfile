# There are a few Dockerfile restrictions when using Github Actions
# See: https://docs.github.com/en/actions/creating-actions/dockerfile-support-for-github-actions

FROM debian:bullseye-slim
ENV DEBIAN_FRONTEND=noninteractive
ENV LC_ALL=C.UTF-8
ENV LANG=C.UTF-8
ARG CONTAINER_USER=esp
ARG CONTAINER_GROUP=esp
ARG RUST_CHANNEL=stable
ARG ESP_BOARD=esp32c3

RUN apt-get update \
    && apt-get install -y git curl ninja-build llvm-dev libclang-dev \
    clang unzip python3 python3-pip pkg-config libssl-dev \
    && apt-get clean -y \
    && rm -rf /var/lib/apt/lists/* /tmp/library-scripts

RUN adduser --disabled-password --gecos "" ${CONTAINER_USER}
USER ${CONTAINER_USER}
WORKDIR /home/${CONTAINER_USER}

RUN curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- \
    --default-toolchain ${RUST_CHANNEL} -y --profile minimal \
    --component rust-src,clippy,rustfmt --target riscv32imc-unknown-none-elf

ENV PATH=${PATH}:$HOME/.cargo/bin

RUN ARCH=$($HOME/.cargo/bin/rustup show | grep "Default host" | sed -e 's/.* //') && \
    curl -L "https://github.com/esp-rs/espflash/releases/latest/download/cargo-espflash-${ARCH}.zip" -o "${HOME}/.cargo/bin/cargo-espflash.zip" && \
    unzip "${HOME}/.cargo/bin/cargo-espflash.zip" -d "${HOME}/.cargo/bin/" && \
    rm "${HOME}/.cargo/bin/cargo-espflash.zip" && \
    chmod u+x "${HOME}/.cargo/bin/cargo-espflash" && \
    curl -L "https://github.com/esp-rs/espflash/releases/latest/download/espflash-${ARCH}.zip" -o "${HOME}/.cargo/bin/espflash.zip" && \
    unzip "${HOME}/.cargo/bin/espflash.zip" -d "${HOME}/.cargo/bin/" && \
    rm "${HOME}/.cargo/bin/espflash.zip" && \
    chmod u+x "${HOME}/.cargo/bin/espflash" && \
    curl -L "https://github.com/esp-rs/esp-web-flash-server/releases/latest/download/web-flash-${ARCH}.zip" -o "${HOME}/.cargo/bin/web-flash.zip" && \
    unzip "${HOME}/.cargo/bin/web-flash.zip" -d "${HOME}/.cargo/bin/" && \
    rm "${HOME}/.cargo/bin/web-flash.zip" && \
    chmod u+x "${HOME}/.cargo/bin/web-flash" && \
    GENERATE_VERSION=$(git ls-remote --refs --sort="version:refname" --tags "https://github.com/cargo-generate/cargo-generate" | cut -d/ -f3- | tail -n1) &&  \
    GENERATE_URL="https://github.com/cargo-generate/cargo-generate/releases/latest/download/cargo-generate-${GENERATE_VERSION}-${ARCH}.tar.gz" &&  \
    curl -L "${GENERATE_URL}" -o "${HOME}/.cargo/bin/cargo-generate.tar.gz" &&  \
    tar xf "${HOME}/.cargo/bin/cargo-generate.tar.gz" -C "${HOME}/.cargo/bin/" &&  \
    rm "${HOME}/.cargo/bin/cargo-generate.tar.gz" && \
    chmod u+x "${HOME}/.cargo/bin/cargo-generate"

CMD "/bin/bash"
