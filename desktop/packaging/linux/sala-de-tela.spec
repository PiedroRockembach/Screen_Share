# Packages the already compiled app for Fedora. Built by
# .github/workflows/linux.yml with:
#   rpmbuild -bb --define "pkg_version 0.1.0" --define "pkg_release 12" \
#            --define "_sourcedir <dir with sala-de-tela, mediamtx, .desktop, .svg>"
%global debug_package %{nil}
%global _build_id_links none

Name:           sala-de-tela
Version:        %{pkg_version}
Release:        %{pkg_release}%{?dist}
Summary:        Transmita a tela para os amigos, sem a voz do Discord
License:        LicenseRef-Proprietary
URL:            https://github.com/PiedroRockembach/Screen_Share
ExclusiveArch:  x86_64

Requires:       gstreamer1-plugins-base
Requires:       gstreamer1-plugins-good
Requires:       gstreamer1-plugins-bad-free
Requires:       pipewire-gstreamer
Requires:       pipewire-utils
Requires:       pulseaudio-utils
Requires:       xdg-desktop-portal
# Encodes and decodes H.264 on the CPU when the video card can't; Fedora's
# own repositories carry OpenH264 but not x264 or libav's H.264.
Recommends:     gstreamer1-plugin-openh264

%description
Substituto gratuito do "Go Live" do Discord: transmissão de tela em 1080p60
entre amigos, com o som do PC e nunca com a voz da chamada. Cada um assiste
dentro do próprio app, com conexão direta entre os PCs.

%install
install -Dm755 %{_sourcedir}/sala-de-tela %{buildroot}%{_prefix}/lib/sala-de-tela/sala-de-tela
install -Dm755 %{_sourcedir}/mediamtx %{buildroot}%{_prefix}/lib/sala-de-tela/mediamtx
mkdir -p %{buildroot}%{_bindir}
ln -s ../lib/sala-de-tela/sala-de-tela %{buildroot}%{_bindir}/sala-de-tela
install -Dm644 %{_sourcedir}/sala-de-tela.desktop %{buildroot}%{_datadir}/applications/sala-de-tela.desktop
install -Dm644 %{_sourcedir}/sala-de-tela.svg %{buildroot}%{_datadir}/icons/hicolor/scalable/apps/sala-de-tela.svg

%post
# Opens the room port in firewalld's default zone.
if command -v firewall-cmd >/dev/null 2>&1 && firewall-cmd --state >/dev/null 2>&1; then
    firewall-cmd --permanent --add-port=48900/udp >/dev/null 2>&1 && firewall-cmd --reload >/dev/null 2>&1 || :
fi

%postun
if [ "$1" -eq 0 ] && command -v firewall-cmd >/dev/null 2>&1 && firewall-cmd --state >/dev/null 2>&1; then
    firewall-cmd --permanent --remove-port=48900/udp >/dev/null 2>&1 && firewall-cmd --reload >/dev/null 2>&1 || :
fi

%files
%{_bindir}/sala-de-tela
%{_prefix}/lib/sala-de-tela/
%{_datadir}/applications/sala-de-tela.desktop
%{_datadir}/icons/hicolor/scalable/apps/sala-de-tela.svg
