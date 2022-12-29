# Modified from: https://github.com/m1guelpf/plz-cli/blob/main/install.sh

#!/usr/bin/env bash
set -e

main() {
    PLATFORM="$(uname | tr '[:upper:]' '[:lower:]')"
    if [ "$PLATFORM" = "mingw32_nt" ] || [ "$PLATFORM" = "mingw64_nt" ]; then
        PLATFORM="windows"
    fi

    BIN_DIR=${BIN_DIR:-$HOME/.local/bin}

    case $SHELL in
    */zsh)
        PROFILE=$HOME/.zshrc
        ;;
    */bash)
        PROFILE=$HOME/.bashrc
        ;;
    */fish)
        PROFILE=$HOME/.config/fish/config.fish
        ;;
    */ash)
        PROFILE=$HOME/.profile
        ;;
    *)
        echo "could not detect shell, manually add ${BIN_DIR} to your PATH."
        exit 1
    esac

    if [[ ":$PATH:" != *":${BIN_DIR}:"* ]]; then
        echo >> "$PROFILE" && echo "export PATH=\"\$PATH:$BIN_DIR\"" >> "$PROFILE"
    fi

    ARCHITECTURE="$(uname -m)"
    if [ "${ARCHITECTURE}" = "x86_64" ]; then
    # Redirect stderr to /dev/null to avoid printing errors if non Rosetta.
        if [ "$(sysctl -n sysctl.proc_translated 2>/dev/null)" = "1" ]; then
            ARCHITECTURE="aarch64" # Rosetta.
        else
            ARCHITECTURE="x86_64" # Intel.
        fi
    elif [ "${ARCHITECTURE}" = "arm64" ] ||[ "${ARCHITECTURE}" = "aarch64" ] ; then
        ARCHITECTURE="aarch64" # Arm.
    else
        ARCHITECTURE="x86_64" # Amd.
    fi

    if [[ "$PLATFORM" == "windows" ]]; then
        EXTENSION=".zip"
    else
        EXTENSION=".tar.xz"
    fi

    BINARY_URL="https://github.com/mufeez-amjad/avail/releases/latest/download/avail-${ARCHITECTURE}-${PLATFORM}${EXTENSION}"

    unset exe

    if [ ! -d "$BIN_DIR" ]; then
        mkdir -p "$BIN_DIR"
    fi

    echo "Downloading latest binary from $BINARY_URL to $BIN_DIR"
    if [[ "$PLATFORM" == "windows" ]]; then
        ensure curl -L "$BINARY_URL" -o "$BIN_DIR/avail.zip"
        unzip "$BIN_DIR/avail.zip" -d "$BIN_DIR"
        rm "$BIN_DIR/avail.zip"
        exe = ".exe"
    else
        ensure curl -L "$BINARY_URL" | tar -xJ -C "$BIN_DIR" --strip-components 1
    fi

    if [ ! -f "$BIN_DIR/avail$exe" ]; then
        echo "Download failed, could not find $BIN_DIR/avail$exe"
        exit 1
    fi

    chmod +x "$BIN_DIR/avail$exe"
    echo "installed - $("$BIN_DIR/avail$exe" --version)"
}

# Run a command that should never fail. If the command fails execution
# will immediately terminate with an error showing the failing
# command.
ensure() {
  if ! "$@"; then err "command failed: $*"; fi
}

main "$@" || exit 1
