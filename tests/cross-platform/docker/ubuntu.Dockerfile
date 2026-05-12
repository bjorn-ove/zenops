FROM ubuntu:24.04

ENV DEBIAN_FRONTEND=noninteractive \
    LC_ALL=C.UTF-8 \
    LANG=C.UTF-8 \
    TERM=xterm-256color

RUN apt-get update && apt-get install -y --no-install-recommends \
        bash zsh git curl ca-certificates build-essential pkg-config \
        python3 python3-pexpect locales \
    && locale-gen C.UTF-8 \
    && rm -rf /var/lib/apt/lists/*

RUN useradd --create-home --shell /bin/bash tester
USER tester
WORKDIR /home/tester

ENV PATH=/home/tester/.cargo/bin:/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin \
    CARGO_TARGET_DIR=/tmp/cargo-target

RUN curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs \
        | sh -s -- -y --no-modify-path --default-toolchain stable

RUN git config --global user.name  "Test User" \
 && git config --global user.email "test@example.com" \
 && git config --global init.defaultBranch main
