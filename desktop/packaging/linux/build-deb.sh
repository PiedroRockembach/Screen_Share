#!/bin/bash
# Builds the .deb for Debian, Ubuntu and Mint from an already compiled app.
# Run on the oldest supported base (Debian 12) so the package installs on
# newer ones too.
# Usage: build-deb.sh <version, e.g. 0.1.0-12> <mediamtx binary> <output dir>
set -euo pipefail
here=$(dirname "$(readlink -f "$0")")
desktop=$(readlink -f "$here/../..")
version=$1
mediamtx=$2
out=$3

pkg=$(mktemp -d)
install -Dm755 "$desktop/target/release/sala-de-tela" "$pkg/usr/lib/sala-de-tela/sala-de-tela"
install -Dm755 "$mediamtx" "$pkg/usr/lib/sala-de-tela/mediamtx"
mkdir -p "$pkg/usr/bin"
ln -s ../lib/sala-de-tela/sala-de-tela "$pkg/usr/bin/sala-de-tela"
install -Dm644 "$here/../sala-de-tela.desktop" "$pkg/usr/share/applications/sala-de-tela.desktop"
install -Dm644 "$here/../sala-de-tela.svg" "$pkg/usr/share/icons/hicolor/scalable/apps/sala-de-tela.svg"

# The system libraries the app links against, with the versions it needs.
# GStreamer itself is pinned to 1.22+ below: older ones (Ubuntu 22.04) lack
# videoconvertscale, which every pipeline uses.
work=$(mktemp -d)
mkdir -p "$work/debian"
printf 'Source: sala-de-tela\n\nPackage: sala-de-tela\nArchitecture: any\n' > "$work/debian/control"
shlibs=$(cd "$work" && dpkg-shlibdeps -O "$pkg/usr/lib/sala-de-tela/sala-de-tela" | sed 's/^shlibs:Depends=//')

mkdir -p "$pkg/DEBIAN"
cat > "$pkg/DEBIAN/control" <<EOF
Package: sala-de-tela
Version: $version
Architecture: amd64
Maintainer: PiedroRockembach <PiedroRockembach@users.noreply.github.com>
Homepage: https://github.com/PiedroRockembach/Screen_Share
Section: video
Priority: optional
Depends: $shlibs, libgstreamer1.0-0 (>= 1.22), gstreamer1.0-plugins-base (>= 1.22), gstreamer1.0-plugins-good, gstreamer1.0-plugins-bad, gstreamer1.0-libav, gstreamer1.0-pipewire, pipewire-bin, pulseaudio-utils, xdg-desktop-portal
Recommends: gstreamer1.0-plugins-ugly
Description: Transmita a tela para os amigos, sem a voz do Discord
 Substituto gratuito do "Go Live" do Discord: transmissão de tela em
 1080p60 entre amigos, com o som do PC e nunca com a voz da chamada.
 Cada um assiste dentro do próprio app, com conexão direta entre os PCs.
EOF

# Opens the room port when ufw is active (the app asks again if it isn't).
cat > "$pkg/DEBIAN/postinst" <<'EOF'
#!/bin/sh
set -e
if [ "$1" = configure ] && command -v ufw >/dev/null 2>&1 && ufw status 2>/dev/null | grep -qiE 'status: (active|ativo)'; then
    ufw allow 48900/udp comment 'Sala de Tela' >/dev/null || true
fi
exit 0
EOF
chmod 755 "$pkg/DEBIAN/postinst"

mkdir -p "$out"
dpkg-deb --root-owner-group --build "$pkg" "$out/sala-de-tela_${version}_amd64.deb"
rm -rf "$pkg" "$work"
