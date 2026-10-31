/**
 * The interval under each rating button: when FSRS would show the card
 * again for that rating (`previewRatings`, one `RatingPreview` per rating).
 *
 * Whole days come from `interval_days`. A learning step has `interval_days`
 * 0 and is minutes or hours away, so its length is `due_date` minus the
 * instant the preview was made (read from the injected clock) rather than
 * "0 days". Long intervals are rounded the way Anki shows them (30-day
 * months, 365-day years, one decimal).
 */

import type { MessageKey } from '../i18n'
import type { Rating, RatingPreview } from '../platform/types'

export type IntervalUnit = 'minutes' | 'hours' | 'days' | 'months' | 'years'

/** The message (`{n} min`, ...) of each unit. */
export const INTERVAL_KEYS: Record<IntervalUnit, MessageKey> = {
  minutes: 'interval.minutes',
  hours: 'interval.hours',
  days: 'interval.days',
  months: 'interval.months',
  years: 'interval.years',
}

/** The review buttons, Again to Easy, with their labels. */
export const RATING_KEYS: Record<Rating, MessageKey> = {
  1: 'rating.again',
  2: 'rating.hard',
  3: 'rating.good',
  4: 'rating.easy',
}

export const RATINGS: readonly Rating[] = [1, 2, 3, 4]

export interface IntervalLabel {
  value: number
  unit: IntervalUnit
}

const MINUTE_MS = 60_000

/** The interval `preview` schedules, for a preview made at `previewedAtMs`. */
export function nextInterval(preview: RatingPreview, previewedAtMs: number): IntervalLabel {
  const days = preview.interval_days
  if (days >= 365) return { value: oneDecimal(days / 365), unit: 'years' }
  if (days >= 30) return { value: oneDecimal(days / 30), unit: 'months' }
  if (days >= 1) return { value: days, unit: 'days' }
  const due = Date.parse(preview.due_date)
  const minutes = Number.isNaN(due) ? 1 : Math.max(1, Math.round((due - previewedAtMs) / MINUTE_MS))
  if (minutes >= 24 * 60) return { value: Math.round(minutes / (24 * 60)), unit: 'days' }
  if (minutes >= 60) return { value: Math.round(minutes / 60), unit: 'hours' }
  return { value: minutes, unit: 'minutes' }
}

/** The previews by rating; a rating FSRS gave no preview for is missing. */
export function previewsByRating(previews: RatingPreview[]): Map<Rating, RatingPreview> {
  return new Map(previews.map((preview) => [preview.rating, preview]))
}

function oneDecimal(value: number): number {
  return Math.round(value * 10) / 10
}
