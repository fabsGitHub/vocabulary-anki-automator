/**
 * Turns pipeline warnings into sentences for the approval window (Milestones
 * C and D).
 *
 * The pipeline never fails a capture because a provider failed; it leaves the
 * field empty and reports why (docs/architecture.md § 5). The reason decides
 * what the user should do, so each error kind gets its own sentence, and
 * MyMemory's exhausted daily quota, the one failure the user can fix in the
 * settings, says how.
 */

import type { PipelineWarning } from '../api'

type Translate = (key: string, params?: Record<string, unknown>) => string

/** One warning as shown: the sentence, plus the provider's own message when it sent one. */
export interface WarningText {
  text: string
  detail: string | null
}

/**
 * Formats `warning` with the active locale's `t`; `has` tells whether a
 * message key exists, so unknown provider names are shown as they are.
 */
export function formatWarning(
  warning: PipelineWarning,
  t: Translate,
  has: (key: string) => boolean,
): WarningText {
  const error = warning.error
  const providerKey = `providers.${error.provider}`
  const provider = has(providerKey) ? t(providerKey) : error.provider
  const stageKey = `warnings.stage.${warning.stage}`
  const stage = has(stageKey) ? t(stageKey) : warning.stage

  let reason: string
  switch (error.kind) {
    case 'quota_exceeded':
      reason =
        error.provider === 'mymemory'
          ? t('warnings.mymemoryQuota')
          : t('warnings.kind.quota_exceeded', { provider })
      break
    case 'http':
      reason = t('warnings.kind.http', { provider, status: error.status })
      break
    case 'unsupported':
      reason = t('warnings.kind.unsupported', { provider, what: error.what })
      break
    case 'network':
    case 'malformed':
    case 'not_found':
      reason = t(`warnings.kind.${error.kind}`, { provider })
      break
    default:
      reason = t('warnings.kind.other', { provider })
  }

  const message =
    'message' in error && typeof error.message === 'string' ? error.message.trim() : ''
  return { text: t('warnings.line', { stage, reason }), detail: message || null }
}
