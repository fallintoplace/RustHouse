# Local-only systemd example (experimental)

Install only after building the RustHouse server and provisioning a
\`rusthouse\` system user/group plus an owned \`/var/lib/rusthouse\` local
database. Review the unit for your distro before enabling it.

\`\`\`sh
sudo install -m 0755 target/release/rusthouse-server /usr/local/bin/rusthouse-server
sudo install -m 0644 deploy/systemd/rusthouse-server.service /etc/systemd/system/
sudo systemctl daemon-reload
sudo systemctl enable --now rusthouse-server.service
curl http://127.0.0.1:8123/ping
\`\`\`

The service binds to \`127.0.0.1\` by default and systemd further limits
networking to localhost. \`ReadWritePaths\` permits the storage lock even
though HTTP SQL is read-only. No TLS, authentication, or public ingress is
configured. This unit is a starting example, **not a production HA deployment**.
It does not initialize a database or create the service account for you.
