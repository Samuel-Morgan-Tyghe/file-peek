# Third-party notices

The app build generates `THIRD_PARTY_NOTICES.txt` inside the app bundle from the exact packages in `Cargo.lock`, plus the DOMPurify license in `package-lock.json`.

Some objc2 family crates omit license files from their published Cargo packages. The files under `upstream/` preserve the upstream license at each crate's exact `.cargo_vcs_info.json` revision. These crates permit use under MIT. No network fetch is needed to generate notices after Cargo and npm dependencies have been downloaded.

Those release-era declarations link to the standard MIT license instead of including its full text. `objc2-MIT.txt` preserves the upstream MIT notice from commit `8a5abacd90d5403fc0463459a35826f892b9142e`, alongside each exact release declaration.

File Peek's own code is covered by the repository's root MIT license. Third-party code retains its respective license and copyright notices.
