FROM archlinux:latest

ENV LC_ALL=C.UTF-8 \
    LANG=C.UTF-8 \
    TERM=xterm-256color

RUN pacman -Syu --noconfirm \
 && pacman -S --noconfirm --needed \
        bash zsh git curl base-devel python python-pexpect \
 && pacman -Scc --noconfirm

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
