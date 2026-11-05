/**
 * What the two install actions download, as the consent dialog states it
 * (Milestone E, docs/architecture.md § 8.4).
 *
 * The backend holds the authoritative pins and is what verifies against them;
 * these copies exist so the user can see exactly what they agree to before
 * anything is fetched. The installer checksums belong to one Anki release, so
 * the dialog only lists them when the backend reports that same version.
 */

export interface PinnedFile {
  name: string
  sha256: string
}

export interface PinnedInstaller extends PinnedFile {
  platform: string
}

export const ANKICONNECT_REPOSITORY = 'github.com/FooSoft/anki-connect'

/** The repository's final, archived commit. */
export const ANKICONNECT_COMMIT = '4064fa142785975255457abd6a496015f5b71f38'

/** The add-on files from `plugin/` at {@link ANKICONNECT_COMMIT}. */
export const ANKICONNECT_FILES: readonly PinnedFile[] = [
  {
    name: '__init__.py',
    sha256: '2185b1243b924152cbac7a7668b40c0bf713902619bfc8ede5c42465066d0f05',
  },
  { name: 'edit.py', sha256: '4c6659247322be76c3015b8f8944ce20498186782880a3c8ff14a48d2e3f58b2' },
  { name: 'util.py', sha256: 'cc0e12de6f809d57d40b29f3eb5f0db1eadad93e19f35fc63a2cfe250216c3b9' },
  { name: 'web.py', sha256: 'b519b4c6fefa3d2058df926c43ade1cf590d595655ee3b20afdf7ccd2a053a78' },
]

/** Where the add-on is written, per platform. */
export const ANKICONNECT_ADDON_DIRS: readonly { platform: string; path: string }[] = [
  { platform: 'macOS', path: '~/Library/Application Support/Anki2/addons21/2055492159' },
  { platform: 'Windows', path: '%APPDATA%\\Anki2\\addons21\\2055492159' },
  { platform: 'Linux', path: '~/.local/share/Anki2/addons21/2055492159' },
]

/** The Anki release the checksums below belong to. */
export const ANKI_PINNED_VERSION = '24.06.3'

/** From the release's signed `anki-24.06.3-checksums.txt`. */
export const ANKI_INSTALLERS: readonly PinnedInstaller[] = [
  {
    platform: 'macOS (Apple Silicon)',
    name: 'anki-24.06.3-mac-apple-qt6.dmg',
    sha256: 'ce2f728c98ab3711450fbc066b8fbe27e983684e5d6195be5f451475d1a48d35',
  },
  {
    platform: 'macOS (Intel)',
    name: 'anki-24.06.3-mac-intel-qt6.dmg',
    sha256: '51045da7f1a189119fb0117e995ea108aa4440616ffc8f43f4a4ed735a7befaa',
  },
  {
    platform: 'Windows (x86_64)',
    name: 'anki-24.06.3-windows-qt6.exe',
    sha256: 'e11ade8b94dbb5374102669ea589bc1072ea07b93897f3b36fa045f5192ef0ba',
  },
]

/** The GitHub release page of an Anki version. */
export function ankiReleaseUrl(version: string): string {
  return `github.com/ankitects/anki/releases/tag/${version}`
}
