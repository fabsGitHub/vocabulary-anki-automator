import { describe, expect, it } from 'vitest'

import type { RatingPreview } from '../platform/types'
import { lesson, lessonCard, NOW } from '../test/fixtures'
import { nextInterval, previewsByRating } from './intervals'
import { lessonNumbers, lessonSteps, splitCloze } from './lessons'

const preview = (interval_days: number, dueInMs: number): RatingPreview => ({
  rating: 3,
  interval_days,
  due_date: new Date(NOW + dueInMs).toISOString(),
})

const MINUTE = 60_000
const DAY = 86_400_000

describe('nextInterval', () => {
  it('counts a learning step from its due date, in minutes or hours', () => {
    expect(nextInterval(preview(0, 10 * MINUTE), NOW)).toEqual({ value: 10, unit: 'minutes' })
    expect(nextInterval(preview(0, 20_000), NOW)).toEqual({ value: 1, unit: 'minutes' })
    expect(nextInterval(preview(0, -5 * MINUTE), NOW)).toEqual({ value: 1, unit: 'minutes' })
    expect(nextInterval(preview(0, 90 * MINUTE), NOW)).toEqual({ value: 2, unit: 'hours' })
    expect(nextInterval(preview(0, 30 * 60 * MINUTE), NOW)).toEqual({ value: 1, unit: 'days' })
  })

  it('uses the scheduled days once there are any', () => {
    expect(nextInterval(preview(1, DAY), NOW)).toEqual({ value: 1, unit: 'days' })
    expect(nextInterval(preview(29, 29 * DAY), NOW)).toEqual({ value: 29, unit: 'days' })
    expect(nextInterval(preview(45, 45 * DAY), NOW)).toEqual({ value: 1.5, unit: 'months' })
    expect(nextInterval(preview(400, 400 * DAY), NOW)).toEqual({ value: 1.1, unit: 'years' })
  })

  it('counts a learning step from the instant the preview was made', () => {
    const due = preview(0, 10 * MINUTE)
    expect(nextInterval(due, NOW + 5 * MINUTE)).toEqual({ value: 5, unit: 'minutes' })
  })

  it('survives a due date it cannot parse', () => {
    expect(nextInterval({ rating: 1, interval_days: 0, due_date: 'soon' }, NOW)).toEqual({
      value: 1,
      unit: 'minutes',
    })
  })

  it('finds the preview of each rating', () => {
    const map = previewsByRating([preview(3, 3 * DAY), { ...preview(0, MINUTE), rating: 1 }])
    expect(map.get(3)?.interval_days).toBe(3)
    expect(map.get(1)?.interval_days).toBe(0)
    expect(map.has(2)).toBe(false)
  })
})

describe('lessons', () => {
  it('numbers lessons within their language pair, in list order', () => {
    const lessons = [
      lesson({ id: 'a', source_lang: 'en', target_lang: 'de' }),
      lesson({ id: 'b', source_lang: 'zh-CN', target_lang: 'en' }),
      lesson({ id: 'c', source_lang: 'en', target_lang: 'de' }),
      lesson({ id: 'd', source_lang: 'de', target_lang: 'en' }),
      lesson({ id: 'e', source_lang: 'zh-CN', target_lang: 'en' }),
    ]
    expect(lessonNumbers(lessons)).toEqual([1, 1, 2, 1, 2])
  })

  it('steps through every card in context, then as a cloze', () => {
    const first = lessonCard({ card_id: 1 })
    const second = lessonCard({ card_id: 2 })
    const steps = lessonSteps(lesson({ cards: [first, second] }))
    expect(steps.map((step) => `${step.kind}:${step.card.card_id}`)).toEqual([
      'context:1',
      'cloze:1',
      'context:2',
      'cloze:2',
    ])
  })

  it('splits a cloze at its first gap', () => {
    expect(splitCloze('Finding it was pure ____.')).toEqual({
      before: 'Finding it was pure ',
      after: '.',
    })
    expect(splitCloze('____ and ____')).toEqual({ before: '', after: ' and ____' })
    expect(splitCloze('no gap')).toBeNull()
  })
})
