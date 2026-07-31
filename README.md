<div align="center">
  <img src="assets/logo.png" alt="Donut Browser Logo" width="150">
  <h1>Donut Browser</h1>
  <strong>Open Source Anti-Detect Browser</strong>
  <br>
  <a href="https://donutbrowser.com">donutbrowser.com</a>
</div>
<br>

<p align="center">
  <a style="text-decoration: none;" href="https://github.com/sondecode/donutbrowser/releases/latest" target="_blank"><img alt="GitHub release" src="https://img.shields.io/github/v/release/sondecode/donutbrowser">
  </a>
  <a style="text-decoration: none;" href="https://github.com/sondecode/donutbrowser/issues" target="_blank">
    <img src="https://img.shields.io/badge/PRs-welcome-brightgreen.svg?style=flat" alt="PRs Welcome">
  </a>
  <a style="text-decoration: none;" href="https://github.com/sondecode/donutbrowser/blob/main/LICENSE" target="_blank">
    <img src="https://img.shields.io/badge/license-AGPL--3.0-blue.svg" alt="License">
  </a>
</p>

> This is a fork of [zhom/donutbrowser](https://github.com/zhom/donutbrowser).
> Releases here are built for **macOS and Windows only**, and are **not
> code-signed** — see [Install](#install) for the Gatekeeper and SmartScreen
> steps.

<img alt="Donut Browser Preview" src="assets/donut-preview.png" />

## Features

- **Unlimited browser profiles**: each fully isolated with its own fingerprint, cookies, extensions, and data
- **Anti-detect Chromium engine**: powered by [Wayfern](https://wayfern.com), which is privacy-focused Chromium fork that comes with advanced fingerprint spoofing which naturally hides information in a way that is not detected by Cloudflare, reCaptcha v3, and other browser fingerprinting and anti-bot services.
- **DNS AdBlocker** - block ads, trackers, and other unwanted content with per-profile DNS blocking
- **Proxy support**: HTTP, HTTPS, SOCKS4, SOCKS5 per profile, with dynamic proxy URLs
- **VPN support**: WireGuard configs per profile
- **Local API & MCP**: REST API and [Model Context Protocol](https://modelcontextprotocol.io) server for integration with Claude, automation tools, and custom workflows
- **Profile groups**: organize profiles and apply bulk settings
- **Import profiles**: migrate from Chrome, Edge, Brave, or other Chromium browsers
- **Cookie & extension management**: import/export cookies, manage extensions per profile
- **Default browser**: set Donut as your default browser and choose which profile opens each link
- **Cloud sync**: sync profiles, proxies, and groups across devices (self-hostable)
- **E2E encryption**: optional end-to-end encrypted sync with a password only you know
- **Zero telemetry**: no tracking or device fingerprinting

## Install

Grab the assets from the [latest release](https://github.com/sondecode/donutbrowser/releases/latest).
Filenames carry the version, e.g. `Donut_0.29.0_aarch64.dmg`.

### macOS

| Architecture | Asset |
|---|---|
| Apple Silicon | `Donut_<version>_aarch64.dmg` |
| Intel | `Donut_<version>_x64.dmg` |

These builds are ad-hoc signed but **not notarized**, so Gatekeeper refuses to
open them on first launch. After dragging the app into `/Applications`, clear
the quarantine flag once:

```bash
xattr -dr com.apple.quarantine /Applications/Donut.app
```

### Windows

| Format | Asset |
|---|---|
| Installer (x64) | `Donut_<version>_x64-setup.exe` |
| Portable (x64) | `Donut_<version>_x64-portable.zip` |

The installer is **unsigned**, so SmartScreen warns about an unknown publisher.
Choose **More info** → **Run anyway**.

### Linux

Not built by this fork. Use the [upstream releases](https://github.com/zhom/donutbrowser/releases/latest),
which ship deb, rpm and AppImage packages.

## Self-Hosting Sync

Donut Browser supports syncing profiles, proxies, and groups across devices via a self-hosted sync server, which makes sync completely free. See the [Self-Hosting Donut Sync guide](https://donutbrowser.com/docs/self-hosting) for Docker-based setup instructions.

## Development

See [CONTRIBUTING.md](CONTRIBUTING.md).

## Community

- **Issues with this fork**: [GitHub Issues](https://github.com/sondecode/donutbrowser/issues)
- **Upstream project**: [Issues](https://github.com/zhom/donutbrowser/issues) · [Discussions](https://github.com/zhom/donutbrowser/discussions)

## Contributors

Contributors to the upstream project this fork is based on. The list is no
longer auto-generated here, so it reflects upstream as of the fork point.

<!-- readme: collaborators,contributors -start -->
<table>
	<tbody>
		<tr>
            <td align="center">
                <a href="https://github.com/zhom">
                    <img src="https://avatars.githubusercontent.com/u/2717306?v=4" width="100;" alt="zhom"/>
                    <br />
                    <sub><b>zhom</b></sub>
                </a>
            </td>
            <td align="center">
                <a href="https://github.com/HassiyYT">
                    <img src="https://avatars.githubusercontent.com/u/81773493?v=4" width="100;" alt="HassiyYT"/>
                    <br />
                    <sub><b>Hassiy</b></sub>
                </a>
            </td>
            <td align="center">
                <a href="https://github.com/xenos1337">
                    <img src="https://avatars.githubusercontent.com/u/66328734?v=4" width="100;" alt="xenos1337"/>
                    <br />
                    <sub><b>xenos</b></sub>
                </a>
            </td>
            <td align="center">
                <a href="https://github.com/webees">
                    <img src="https://avatars.githubusercontent.com/u/5155291?v=4" width="100;" alt="webees"/>
                    <br />
                    <sub><b>JockLee</b></sub>
                </a>
            </td>
            <td align="center">
                <a href="https://github.com/yb403">
                    <img src="https://avatars.githubusercontent.com/u/87396571?v=4" width="100;" alt="yb403"/>
                    <br />
                    <sub><b>yb403</b></sub>
                </a>
            </td>
            <td align="center">
                <a href="https://github.com/huy97">
                    <img src="https://avatars.githubusercontent.com/u/30153437?v=4" width="100;" alt="huy97"/>
                    <br />
                    <sub><b>Huy Le</b></sub>
                </a>
            </td>
		</tr>
		<tr>
            <td align="center">
                <a href="https://github.com/drunkod">
                    <img src="https://avatars.githubusercontent.com/u/9677471?v=4" width="100;" alt="drunkod"/>
                    <br />
                    <sub><b>drunkod</b></sub>
                </a>
            </td>
            <td align="center">
                <a href="https://github.com/JorySeverijnse">
                    <img src="https://avatars.githubusercontent.com/u/117462355?v=4" width="100;" alt="JorySeverijnse"/>
                    <br />
                    <sub><b>Jory Severijnse</b></sub>
                </a>
            </td>
            <td align="center">
                <a href="https://github.com/ThiagoMafra-Integrare">
                    <img src="https://avatars.githubusercontent.com/u/222241596?v=4" width="100;" alt="ThiagoMafra-Integrare"/>
                    <br />
                    <sub><b>Thiago Mafra</b></sub>
                </a>
            </td>
            <td align="center">
                <a href="https://github.com/liasica">
                    <img src="https://avatars.githubusercontent.com/u/671431?v=4" width="100;" alt="liasica"/>
                    <br />
                    <sub><b>liasica</b></sub>
                </a>
            </td>
		</tr>
	<tbody>
</table>
<!-- readme: collaborators,contributors -end -->

## Contact

Have an urgent question or want to report a security vulnerability? Send an email to [contact@donutbrowser.com](mailto:contact@donutbrowser.com).

## License

This project is licensed under the AGPL-3.0 License - see the [LICENSE](LICENSE) file for details.
