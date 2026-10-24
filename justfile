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
    # Absolute Exec paths: the panel and D-Bus may not have {{bindir}} in PATH.
    mkdir -p "{{dest-sharedir}}/applications"
    sed 's|^Exec=|Exec={{bindir}}/|' res/{{appid}}.desktop > "{{dest-sharedir}}/applications/{{appid}}.desktop"
    sed 's|^Exec=|Exec={{bindir}}/|' res/{{appid}}.Applet.desktop > "{{dest-sharedir}}/applications/{{appid}}.Applet.desktop"
    install -Dm0644 res/{{appid}}.svg "{{dest-sharedir}}/icons/hicolor/scalable/apps/{{appid}}.svg"
    install -Dm0644 res/{{appid}}-symbolic.svg "{{dest-sharedir}}/icons/hicolor/symbolic/apps/{{appid}}-symbolic.svg"
    install -Dm0644 res/{{appid}}.metainfo.xml "{{dest-sharedir}}/metainfo/{{appid}}.metainfo.xml"
    mkdir -p "{{dest-sharedir}}/dbus-1/services"
    sed 's|@bindir@|{{bindir}}|' res/{{appid}}.service.in > "{{dest-sharedir}}/dbus-1/services/{{appid}}.service"

uninstall:
    -pkill -f '/{{name}}$'
    rm -f "{{dest-bindir}}/{{name}}" "{{dest-bindir}}/{{name}}-applet"
    rm -f "{{dest-sharedir}}/applications/{{appid}}.desktop" "{{dest-sharedir}}/applications/{{appid}}.Applet.desktop"
    # Earlier versions started from autostart.
    rm -f "{{dest-autostartdir}}/{{appid}}.desktop"
    rm -f "{{dest-sharedir}}/icons/hicolor/scalable/apps/{{appid}}.svg"
    rm -f "{{dest-sharedir}}/icons/hicolor/symbolic/apps/{{appid}}-symbolic.svg"
    rm -f "{{dest-sharedir}}/metainfo/{{appid}}.metainfo.xml"
    rm -f "{{dest-sharedir}}/dbus-1/services/{{appid}}.service"

# Builds a .deb into target/debian, from the release build.
deb: build-release
    #!/usr/bin/env bash
    set -euo pipefail
    umask 022
    stage="target/debian/{{name}}"
    rm -rf "$stage" && mkdir -p "$stage/DEBIAN"
    just rootdir="$PWD/$stage" prefix=/usr install
    install -Dm0644 res/deb/copyright "$stage/usr/share/doc/{{name}}/copyright"
    version=$(cargo metadata --no-deps --format-version 1 | python3 -c 'import json,sys; print(json.load(sys.stdin)["packages"][0]["version"])')
    arch=$(dpkg --print-architecture)
    # dpkg-shlibdeps wants a debian/control to sit next to; give it a minimal one.
    shlibs=$(mktemp -d) && mkdir "$shlibs/debian" && printf 'Source: {{name}}\n\nPackage: {{name}}\nArchitecture: any\n' > "$shlibs/debian/control"
    depends=$(cd "$shlibs" && dpkg-shlibdeps -O -e "$OLDPWD/$stage/usr/bin/{{name}}" -e "$OLDPWD/$stage/usr/bin/{{name}}-applet" 2>/dev/null | sed 's/^shlibs:Depends=//')
    rm -rf "$shlibs"
    size=$(du -sk --exclude=DEBIAN "$stage" | cut -f1)
    sed -e "s|@version@|$version|" -e "s|@arch@|$arch|" -e "s|@depends@|$depends|" -e "s|@size@|$size|" res/deb/control.in > "$stage/DEBIAN/control"
    dpkg-deb --root-owner-group --build "$stage" "target/debian/{{name}}_${version}_${arch}.deb"

# Writes the app's folder for a pull request to pop-os/cosmic-flatpak into
# target/cosmic-flatpak, with the manifest pointing at HEAD (push it first).
flatpak-pr:
    #!/usr/bin/env bash
    set -euo pipefail
    commit=$(git rev-parse HEAD)
    if ! git branch -r --contains "$commit" | grep -q .; then
        echo "Push $commit to GitHub first: the Flatpak is built from it." >&2
        exit 1
    fi
    dir="target/cosmic-flatpak/app/{{appid}}"
    rm -rf "$dir" && mkdir -p "$dir"
    sed "s|@commit@|$commit|" flatpak/{{appid}}.json > "$dir/{{appid}}.json"
    cp flatpak/cargo-sources.json "$dir/"
    echo "$dir"
