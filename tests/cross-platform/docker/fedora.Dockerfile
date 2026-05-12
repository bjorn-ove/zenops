FROM fedora:40

ENV LC_ALL=C.UTF-8 \
    LANG=C.UTF-8 \
    TERM=xterm-256color

RUN dnf install -y --setopt=install_weak_deps=False \
        bash zsh git curl ca-certificates gcc make pkgconf-pkg-config \
        python3 python3-pexpect glibc-langpack-en which \
    && dnf clean all

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
