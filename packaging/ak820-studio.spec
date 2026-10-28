%global debug_package %{nil}

Name:           ak820-studio
Version:        0.1.5
Release:        1%{?dist}
Summary:        Screen studio and clock sync for the AJAZZ / Epomaker AK820 Pro

License:        MIT
URL:            https://github.com/WhatTFF/linux-ajazz-ak820-pro-app
Source0:        %{url}/archive/v%{version}/linux-ajazz-ak820-pro-app-%{version}.tar.gz

BuildRequires:  rust-packaging >= 21
BuildRequires:  gtk3-devel
BuildRequires:  webkit2gtk4.1-devel
BuildRequires:  libsoup3-devel
BuildRequires:  librsvg2-devel
BuildRequires:  openssl-devel
BuildRequires:  pkgconf-pkg-config

Requires:       webkit2gtk4.1
Requires:       hicolor-icon-theme

%description
Put GIFs, images, video, text, drawings and procedural animations on the
128x128 screen of the AJAZZ / Epomaker AK820 Pro, and keep the keyboard clock in
sync with the system clock on boot, on plug-in, after suspend and after
timezone changes.

Talks to the keyboard directly over USB (hidraw); no vendor driver and no
browser are involved. Unofficial, not affiliated with AJAZZ or Epomaker.

%prep
%autosetup -n linux-ajazz-ak820-pro-app-%{version}

%generate_buildrequires
%cargo_generate_buildrequires

%build
%cargo_build -p ak820-studio -p ak820-cli

%install
install -Dm755 target/release/ak820-studio %{buildroot}%{_bindir}/ak820-studio
install -Dm755 target/release/ak820ctl %{buildroot}%{_bindir}/ak820ctl
install -Dm644 packaging/ak820-studio.desktop %{buildroot}%{_datadir}/applications/ak820-studio.desktop
install -Dm644 packaging/io.github.whattff.Ak820Studio.metainfo.xml %{buildroot}%{_metainfodir}/io.github.whattff.Ak820Studio.metainfo.xml
install -Dm644 apps/desktop/icons/icon.png %{buildroot}%{_datadir}/icons/hicolor/512x512/apps/io.github.whattff.Ak820Studio.png
install -Dm644 apps/desktop/icons/128x128.png %{buildroot}%{_datadir}/icons/hicolor/128x128/apps/io.github.whattff.Ak820Studio.png
install -Dm644 udev/70-ak820.rules %{buildroot}%{_udevrulesdir}/70-ak820.rules
install -Dm644 systemd/ak820-clock.service %{buildroot}%{_userunitdir}/ak820-clock.service
install -Dm644 systemd/ak820-clock.timer %{buildroot}%{_userunitdir}/ak820-clock.timer
install -Dm644 systemd/ak820-connect.service %{buildroot}%{_userunitdir}/ak820-connect.service
install -Dm644 systemd/ak820-resume.service %{buildroot}%{_userunitdir}/ak820-resume.service

%files
%license LICENSE NOTICE
%doc README.md PLAN.md PROTOCOL.md docs/UPSTREAM.md
%{_bindir}/ak820-studio
%{_bindir}/ak820ctl
%{_datadir}/applications/ak820-studio.desktop
%{_metainfodir}/io.github.whattff.Ak820Studio.metainfo.xml
%{_datadir}/icons/hicolor/512x512/apps/io.github.whattff.Ak820Studio.png
%{_datadir}/icons/hicolor/128x128/apps/io.github.whattff.Ak820Studio.png
%{_udevrulesdir}/70-ak820.rules
%{_userunitdir}/ak820-clock.service
%{_userunitdir}/ak820-clock.timer
%{_userunitdir}/ak820-connect.service
%{_userunitdir}/ak820-resume.service

%post
# Let the new hidraw rule apply to a keyboard that is already plugged in.
udevadm control --reload >/dev/null 2>&1 || :
udevadm trigger --subsystem-match=hidraw --action=change >/dev/null 2>&1 || :
echo "AK820 Studio: enable clock autosync with:"
echo "  systemctl --user enable --now ak820-clock.timer"
echo "  systemctl --user enable ak820-resume.service"

%postun
udevadm control --reload >/dev/null 2>&1 || :

%changelog
* Tue Oct 28 2026 WhatTFF <whattff@users.noreply.github.com> - 0.1.5-1
- Initial package
