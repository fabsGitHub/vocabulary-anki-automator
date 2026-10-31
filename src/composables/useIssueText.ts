/** The translated validation message for one settings field, for the tabs that show them inline. */

import { useI18n } from 'vue-i18n'

import { useConfigStore } from './useConfig'

export function useIssueText(): (path: string) => string | null {
  const { t } = useI18n()
  const store = useConfigStore()
  return (path) => {
    const issue = store.issueFor(path)
    return issue ? t(issue.key, issue.params ?? {}) : null
  }
}
