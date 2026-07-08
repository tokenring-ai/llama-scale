%global _build_id_links none

Name:           llama-scale
Version:        %{version}
Release:        1%{?dist}
Summary:        OpenAI-compatible LLM router
License:        MIT
URL:            https://github.com/tokenring-ai/llama-scale
Requires:       ca-certificates
AutoReqProv:    no

%description
A Rust-based OpenAI-compatible LLM router with session affinity
and least-connections load balancing.

%prep
%autosetup -n staging -c -T -D

%install
mkdir -p %{buildroot}/usr/bin
mkdir -p %{buildroot}/usr/lib/systemd/system
mkdir -p %{buildroot}/etc/llama-scale

install -m 0755 usr/bin/llama-scale %{buildroot}/usr/bin/llama-scale
install -m 0644 usr/lib/systemd/system/llama-scale.service \
  %{buildroot}/usr/lib/systemd/system/llama-scale.service
install -m 0644 etc/llama-scale/config.yaml.default \
  %{buildroot}/etc/llama-scale/config.yaml.default

%files
%config(noreplace) /etc/llama-scale/config.yaml.default
/usr/bin/llama-scale
/usr/lib/systemd/system/llama-scale.service

%post
if ! getent passwd llama-scale >/dev/null 2>&1; then
  useradd -r -s /sbin/nologin -d /nonexistent -c "llama-scale service user" llama-scale
fi
install -d -o llama-scale -g llama-scale -m 0750 /var/log/llama-scale
systemctl daemon-reload >/dev/null 2>&1 || true
systemctl enable llama-scale.service >/dev/null 2>&1 || true

%preun
if [ "$1" -eq 0 ]; then
  systemctl disable --now llama-scale.service >/dev/null 2>&1 || true
fi

%postun
systemctl daemon-reload >/dev/null 2>&1 || true