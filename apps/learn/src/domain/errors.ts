/**
 * A human-readable message for a rejected adapter call. The contract says
 * every `PlatformAdapter` method rejects with a string, but a bug, a panic in
 * a Tauri command or a browser API can still surface an `Error` or an
 * object, and the UI must show something readable either way.
 */
export function errorMessage(err: unknown): string {
  if (typeof err === 'string') return err
  if (err instanceof Error) return err.message
  if (err && typeof err === 'object' && 'message' in err) {
    const message = (err as { message: unknown }).message
    if (typeof message === 'string') return message
  }
  try {
    return JSON.stringify(err) ?? String(err)
  } catch {
    return String(err)
  }
}
