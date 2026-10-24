# A build environment for the Tauri crate on Linux, for hosts where the
# GTK/WebKit development headers can't be installed with root access.
#
#   podman build -t tauri-linux -f dev/tauri-linux.Containerfile
#   podman run --rm -v "$PWD:/src:z" -v tauri-cargo:/cargo \
#              -w /src/src-tauri tauri-linux cargo test
#
# Run `npm run build` first: the Tauri context macro reads `../dist`.
FROM fedora:45

# Tauri 2 on Linux links against GTK3 and WebKitGTK 4.1, which is the GTK3
# generation of WebKit ("4.1" is the WebKit ABI, not the GTK version).
RUN dnf install -y \
      gcc gcc-c++ make pkgconf-pkg-config \
      rust cargo \
      gtk3-devel \
      webkit2gtk4.1-devel \
      javascriptcoregtk4.1-devel \
      libsoup3-devel \
      openssl-devel \
      libxdo-devel \
      librsvg2-devel \
      file \
    && dnf clean all

ENV CARGO_HOME=/cargo
ENV RUSTUP_HOME=/cargo/rustup
WORKDIR /src
