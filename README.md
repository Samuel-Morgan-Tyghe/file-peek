# File Peek 🦀

A small, read-only Mac app for taking a quick look at your files. Open Markdown as a formatted page, switch to source, or preview PDFs, images and other documents with macOS Quick Look.

The app and Markdown parser are written in Rust. The interface uses native AppKit controls and WebKit. Files are previewed locally, with no account required.

## Install

**Apple Silicon Macs, macOS 13 or later.** The downloadable build is arm64; Intel Macs can build from source but aren't tested.

1. [Download the latest release](https://github.com/Samuel-Morgan-Tyghe/file-peek/releases/latest).
2. Unzip the `File-Peek-…-macOS-arm64.zip` file.
3. Drag **File Peek.app** into **Applications**, then open it.

Releases are ad-hoc signed and **not Apple-notarized**. macOS may block the first launch. If you trust this download, follow [Apple's instructions for opening an app from an unidentified developer](https://support.apple.com/en-gb/102445) using **System Settings → Privacy & Security**. There is no automatic updater.

## Open files

- Drop a file onto the window or the app's Dock icon.
- Press **⌘O** to choose a file.
- In Finder, right-click a file and choose **Open With → File Peek**.

For double-click Markdown previews, select a `.md` file in Finder, press **⌘I**, choose **File Peek** under **Open with**, then click **Change All…**.

| Action | Shortcut |
| --- | --- |
| Open a file | ⌘O |
| Preview / source | ⌘E |
| Reload after a file changes | ⌘R |
| Larger / smaller text | ⌘+ / ⌘− |
| Reset text size | ⌘0 |

The toolbar also offers **Show in Finder**. Light and dark appearance follow macOS.

## Supported previews

- **Markdown:** headings, emphasis, lists, tables, task lists, strikethrough, fenced code, links and local images. `.md`, `.markdown`, `.mdown`, `.mkd` and `.mdx` are recognized. MDX components aren't executed.
- **Text and source:** plain text, JSON, YAML, CSV, logs and common code files, including Rust. UTF-8 and BOM-marked UTF-16/32 are supported.
- **Other documents:** PDFs, images and formats supported by your installed Quick Look providers.

Text previews are limited to **5 MB**; Markdown images to **20 MB each**. Local images and document links stay within the open document's directory, including subdirectories. Files outside that directory must be opened explicitly. Use ⌘R to pick up changes; previews don't automatically refresh.

## Privacy and file safety

Files are read-only. File Peek has no analytics, accounts or upload service. Markdown is parsed locally on a worker thread. Its HTML is sanitized with DOMPurify; embedded scripts and remote images are blocked. Clicking an external link opens your browser or mail app. Other file formats are handled by the Mac's Quick Look providers.

## Build

Requires macOS, Apple's Command Line Tools, **Rust 1.87+**, **Node 24**, and **Python 3**. CI uses Rust 1.96.1 on an Apple Silicon macOS runner.

```sh
git clone https://github.com/Samuel-Morgan-Tyghe/file-peek.git
cd file-peek
npm ci --ignore-scripts
npm run build
npm test
```

The app is built at `build/File Peek.app`. Both Cargo and npm dependencies are locked. Node fetches the HTML sanitizer at build time; the installed app doesn't need Node or an internet connection.

```sh
cargo fmt --check
cargo clippy --locked --all-targets -- -D warnings
bash scripts/package-release.sh
```

Packaging creates an app ZIP and `SHA256SUMS.txt` in `build/release/`. The ZIP includes the app's own license and third-party notices. To verify a downloaded ZIP, place its checksum file alongside it and run `shasum -a 256 -c SHA256SUMS.txt`.

## Tests and implementation

`cargo test` covers text decoding, Markdown output, read limits, and directory traversal and symlink boundaries. `npm test` also runs the built Rust app's hidden `--self-test` mode against real WebKit, covering sanitization, tables, tasks, code, Unicode, links and local image loading. Run it from the repository root.

- [objc2](https://github.com/madsmtm/objc2) connects Rust to AppKit, WebKit and Quick Look.
- [pulldown-cmark](https://github.com/pulldown-cmark/pulldown-cmark) parses Markdown in Rust.
- [DOMPurify](https://github.com/cure53/DOMPurify) sanitizes the rendered HTML; a small JavaScript layer displays it.

## License

[MIT](LICENSE). Third-party code retains its own licenses; see [license notice generation](licenses/README.md).
