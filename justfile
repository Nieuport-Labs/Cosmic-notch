name := 'cosmic-ext-island'
appid := 'io.github.jiricepelka.CosmicExtIsland'

# Install into ~/.local by default. For a system install: `sudo just prefix=/usr install`.
# Packagers stage the files elsewhere with `just rootdir=debian/tmp prefix=/usr install`.
rootdir := ''
prefix := env_var('HOME') / '.local'
user-install := if prefix == env_var('HOME') / '.local' { 'true' } else { 'false' }

# Where the files end up on the running system, as written into them.
bindir := prefix / 'bin'
sharedir := prefix / 'share'
autostartdir := if user-install == 'true' { env_var('HOME') / '.config/autostart' } else { '/etc/xdg/autostart' }

# Where they are installed now.
dest-bindir := rootdir + bindir
dest-sharedir := rootdir + sharedir
dest-autostartdir := rootdir + autostartdir

default: build-release

build-release *args:
    cargo build --release {{args}}

check:
    cargo clippy --all-targets -- -D warnings
    cargo test

# Runs the island from the build tree, e.g. `just run --expand=calendar`.
run *args:
    RUST_LOG=warn,cosmic_ext_island=debug cargo run --bin {{name}} -- {{args}}

install:
    # Builds as the user; a root or staged install takes what was built before.
    if [ -z "{{rootdir}}" ] && [ "$(id -u)" != 0 ]; then cargo build --release; fi
    install -Dm0755 target/release/{{name}} "{{dest-bindir}}/{{name}}"
    install -Dm0755 target/release/{{name}}-applet "{{dest-bindir}}/{{name}}-applet"
    # Absolute Exec paths: the panel and autostart may not have {{bindir}} in PATH.
    mkdir -p "{{dest-sharedir}}/applications" "{{dest-autostartdir}}"
    sed 's|^Exec=|Exec={{bindir}}/|' res/{{appid}}.desktop > "{{dest-sharedir}}/applications/{{appid}}.desktop"
    sed 's|^Exec=|Exec={{bindir}}/|' res/{{appid}}.Applet.desktop > "{{dest-sharedir}}/applications/{{appid}}.Applet.desktop"
    sed 's|^Exec=|Exec={{bindir}}/|' res/{{appid}}.desktop > "{{dest-autostartdir}}/{{appid}}.desktop"
    install -Dm0644 res/{{appid}}-symbolic.svg "{{dest-sharedir}}/icons/hicolor/symbolic/apps/{{appid}}-symbolic.svg"
    install -Dm0644 res/{{appid}}.metainfo.xml "{{dest-sharedir}}/metainfo/{{appid}}.metainfo.xml"
    mkdir -p "{{dest-sharedir}}/dbus-1/services"
    sed 's|@bindir@|{{bindir}}|' res/{{appid}}.service.in > "{{dest-sharedir}}/dbus-1/services/{{appid}}.service"

uninstall:
    -pkill -f '/{{name}}$'
    rm -f "{{dest-bindir}}/{{name}}" "{{dest-bindir}}/{{name}}-applet"
    rm -f "{{dest-sharedir}}/applications/{{appid}}.desktop" "{{dest-sharedir}}/applications/{{appid}}.Applet.desktop"
    rm -f "{{dest-autostartdir}}/{{appid}}.desktop"
    rm -f "{{dest-sharedir}}/icons/hicolor/symbolic/apps/{{appid}}-symbolic.svg"
    rm -f "{{dest-sharedir}}/metainfo/{{appid}}.metainfo.xml"
    rm -f "{{dest-sharedir}}/dbus-1/services/{{appid}}.service"

# Builds a .deb into target/debian (needs debhelper; Rust comes from rustup, so build deps are not checked).
deb:
    dpkg-buildpackage -b -us -uc -d
    mkdir -p target/debian
    mv ../{{name}}_*.deb ../{{name}}_*.buildinfo ../{{name}}_*.changes target/debian/
