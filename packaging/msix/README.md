# MSIX package (Microsoft Store)

FlexFlyout's package for the Microsoft Store. The same `flexflyout.exe` as in the zip; from a package it:

- keeps the settings and the logs in the package's folder, `%LOCALAPPDATA%\Packages\<package family name>\LocalState` (Windows redirects a packaged app's writes to `%APPDATA%` and `%LOCALAPPDATA%` into a private copy, which Notepad, opened from the tray, would not see);
- starts with Windows through the package's startup task (`windows.startupTask` in the manifest), not the Run key (writes to `HKCU` are redirected too, and the program's path changes with every update);
- does not look for new versions (the Store brings them), and the settings window hides that option.

## Building

The release workflow (`.github/workflows/release.yml`) builds the package on every tag, and when run by hand. The artifact `flexflyout-msix` holds:

- `flexflyout-vX.Y.Z-x64.msix` and `-arm64.msix`: for the Store (unsigned; the Store signs what it publishes), each built on a machine of its architecture;
- `flexflyout-vX.Y.Z-x64-test.msix`, `-arm64-test.msix` and their `-test.cer`: copies signed with a throwaway self-signed certificate, for trying the packages;
- `wack-report-x64.xml`: the Windows App Certification Kit's report on the x64 package (the checks Partner Center runs; GitHub's ARM64 machines do not have the kit).

On Windows with the Windows SDK, the same by hand:

```powershell
cargo build --release -p flexflyout
./packaging/msix/build.ps1 -Exe target/release/flexflyout.exe -Version 0.9.0 -OutDir dist -TestSign
```

The images in `Assets/` are rendered from `crates/flexflyout/assets/flexflyout.svg`, each size on its own (scale-100 to scale-400, and exact sizes for the taskbar). `build.ps1` indexes them with makepri so that Windows picks the right one.

## Trying the test copy

1. Trust the certificate (as an administrator): `Import-Certificate -FilePath flexflyout-vX.Y.Z-x64-test.cer -CertStoreLocation Cert:\LocalMachine\TrustedPeople`
2. Install, from a signed-in session (over SSH it fails with 0x80070005, access denied): `Add-AppxPackage flexflyout-vX.Y.Z-x64-test.msix`
3. Start it from the Start menu, or `explorer.exe "shell:AppsFolder\<package family name>!FlexFlyout"`.
4. Remove it: `Get-AppxPackage *FlexFlyout* | Remove-AppxPackage`, then the certificate from `Cert:\LocalMachine\TrustedPeople`.

## Publishing on the Store

1. Open a developer account at <https://storedeveloper.microsoft.com> (free for individuals; identity check with an ID and a selfie).
2. In Partner Center, reserve the name **FlexFlyout**.
3. From the app's **Product identity**, set the repository variables that the workflow reads, so that the package carries the Store's identity:

   ```sh
   gh variable set MSIX_IDENTITY_NAME --body "<Package/Identity/Name>"
   gh variable set MSIX_PUBLISHER --body "<Package/Identity/Publisher>"   # CN=...
   gh variable set MSIX_PUBLISHER_DISPLAY_NAME --body "<Package/Properties/PublisherDisplayName>"
   ```

4. Run the release workflow (or tag a release), and upload `flexflyout-vX.Y.Z-x64.msix` and `-arm64.msix` from the `flexflyout-msix` artifact to the submission's **Packages** (after the first submission, the release workflow does this through the Store's API).
5. The rest of the submission (the values and the listing texts in English and Japanese are in [`store-listing.ja.md`](store-listing.ja.md), in Japanese for the Japanese Partner Center):
   - **Pricing**: free. **Properties**: category (Utilities & tools), privacy policy URL `https://github.com/akivajp/flexflyout#privacy`.
   - **Age ratings**: the questionnaire (no user content, no purchases).
   - **Store listings** in English and Japanese: description, screenshots (`docs/images/`), and the features.
   - **Restricted capability `runFullTrust`**: a desktop app (a keyboard hook for the volume and media keys, Core Audio, a tray icon, a topmost window that takes no focus).
   - **Notes for certification**: change the volume or press a media key, or play something in Media Player or a browser; the flyout appears in the top right corner. The settings are in the tray icon's menu.

Each version needs a new package version (the workflow takes it from `Cargo.toml`); the Store's last number stays 0.
