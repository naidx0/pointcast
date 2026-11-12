# Signing the macOS build

Pointcast's macOS releases are signed with one self-signed certificate, common name `maxnaid`. It costs nothing and
needs no Apple Developer account. It exists for one reason: so that an update does not make macOS ask for Screen
Recording and Accessibility again.

## Why it matters

macOS remembers a permission against the app's *designated requirement*, a rule read from its signature.

- A build signed ad hoc (`signingIdentity: "-"`, the default in `tauri.conf.json`) has the requirement
  `cdhash H"..."`: this exact binary and no other. Every new version is a stranger, and the permissions have to be
  given again after each update.
- A build signed with the certificate has the requirement
  `identifier "dev.pointcast.app" and certificate leaf = H"<the certificate's SHA-1>"`. Every version signed with the
  same certificate meets it, so macOS keeps what it was allowed.

`install.sh` reads the requirement of the installed app before it replaces it and checks the new one against it
(`codesign --verify -R`). When the new version meets it, nothing is reset and the update ends on `Updated to <version>.`
When it does not (the signer changed), it resets the two permissions with `tccutil` and says macOS may ask again.

When it resets them it also deletes the app's note that it already asked for Screen Recording
(`screen-asked` in the data folder), so the setup assistant offers macOS's prompt again. After a quiet update the
note stays: macOS kept its answer and will not show the prompt a second time, so **Allow** opens the Screen
Recording pane of System Settings instead.

An in-app update runs the `install.sh` built into the version that is *already installed*. So the first release
signed with the certificate is the last one that asks: it replaces an ad hoc build. Every update after it is quiet.

What the certificate does not change: Gatekeeper. To macOS the app is still from an unidentified developer, so a
`.dmg` downloaded in a browser still needs right-click, **Open** the first time. The one-line install is unaffected
(curl does not quarantine what it downloads). Only an Apple Developer ID and notarization remove that warning.

## Make the certificate (once)

On any system with `openssl` and `gh`, writing to a folder outside every repository:

```bash
scripts/make-signing-cert.sh ~/pointcast-signing/pointcast-signing.p12
```

It writes the `.p12` and a `.p12.password` beside it, prints the certificate's SHA-1, and prints the two commands
that hand both to GitHub without showing either:

```bash
openssl base64 -A -in ~/pointcast-signing/pointcast-signing.p12 | gh secret set MAC_SIGN_P12 --repo naidx0/pointcast
gh secret set MAC_SIGN_PASSWORD --repo naidx0/pointcast < ~/pointcast-signing/pointcast-signing.p12.password
```

Keep the two files somewhere that is backed up (a password manager takes both). They never go into the repository,
a log or an artifact. The certificate is valid for ten years.

## What the release does with it

`.github/workflows/release.yml`, on the macOS job, through `scripts/mac-signing.sh`:

1. `setup`: with both secrets present, imports the certificate into a keychain made for the job, signs a probe file
   to prove `codesign` can use it, and names it to the build as `APPLE_SIGNING_IDENTITY` (the certificate's SHA-1).
   With a secret missing it does nothing and the build is ad hoc, as in a fork or in `ci.yml`.
2. Tauri builds and signs the app and the `.dmg`.
3. `check`: prints the designated requirement of the app and of the app inside the `.dmg`. If a certificate was set
   up and the requirement does not name it, the release stops there.
4. `cleanup`: deletes the keychain, also when a step before it failed.

Read the "What was signed" step of a release run: it should show `certificate leaf = H"..."` with the SHA-1 the
script printed when the certificate was made, in lower case.

## If the certificate is lost, or has to be replaced

Nothing breaks. Make a new one with the script (a new path: it refuses to overwrite), set the two secrets again and
release. That one release is signed by a certificate the installed copies do not know, so `install.sh` resets the
two permissions and everyone is asked once more, exactly as in the move from ad hoc. After it, updates are quiet
again. Do the same before the ten years are up. There is nothing to revoke: the certificate is trusted by no Mac,
it only gives the builds a common name.

If the password or the `.p12` may have been seen by someone else, replace it the same way: with it, somebody could
sign a build that macOS would give Pointcast's permissions to, on a Mac where they can already replace the app.

## What is tested, and what is not

`.github/workflows/update-test.yml`, job `signed-update` (run it by hand), on a GitHub macOS runner, with a
certificate made in that run by `make-signing-cert.sh` and set up by the same `mac-signing.sh` the release uses:

- two versions signed with one certificate are different code with the same requirement, and the newer meets the
  older one's requirement while an ad hoc copy of it does not;
- the real `install.sh` over a certificate-signed version leaves the new one running and calls no `tccutil`; over
  an ad hoc version it resets and says so;
- started the way the app starts it (`update.rs`), the installer outlives the process it closes, swaps the app and
  opens the new one: exactly one Pointcast, the new one.

Not tested there: macOS itself keeping a permission. A runner cannot grant Screen Recording, so that is inferred
from the requirement check. See it once on a real Mac:

1. On a Mac with a certificate-signed Pointcast installed and both permissions on (System Settings → Privacy &
   Security → Screen Recording, and Accessibility), run `codesign -d -r- /Applications/Pointcast.app`. It should
   print `certificate leaf = H"..."`, not `cdhash`.
2. Update to the next certificate-signed version from **Settings → General → Check Now → Update**. Pointcast closes
   and opens again by itself, and the setup assistant does not appear.
3. Record a pointer capture. It works without a prompt, both switches are still on in System Settings, and
   `~/Library/Application Support/dev.pointcast.app/update.log` ends on `Updated to <version>.` with no
   `may ask again`.
