import { flushPromises, type VueWrapper } from '@vue/test-utils'
import { afterEach, describe, expect, it } from 'vitest'

import { setLocale } from '../i18n'
import { fakePlatform, mountWith, type FakePlatform } from '../test/fakePlatform'
import * as fx from '../test/fixtures'
import LessonsView from './LessonsView.vue'

const SECOND_CARD = fx.lessonCard({
  card_id: 12,
  word: 'cat',
  translation: 'Katze',
  sentence_source: 'Concatenate the cat.',
  sentence_target: 'Verkette die Katze.',
  cloze: 'Concatenate the ____.',
})

const LESSONS = [
  fx.lesson({ cards: [fx.lessonCard(), SECOND_CARD] }),
  fx.lesson({
    id: 'lesson-20',
    title: 'ZH-CN → EN · Lesson 1',
    source_lang: 'zh-CN',
    target_lang: 'en',
    cards: [
      fx.lessonCard({
        card_id: 20,
        word: '学习',
        translation: 'to study',
        sentence_source: '我在学习中文。',
        sentence_target: 'I am studying Chinese.',
        cloze: '我在____中文。',
        pinyin: 'xué xí',
      }),
    ],
  }),
  fx.lesson({ id: 'lesson-30', title: 'EN → DE · Lesson 2' }),
]

let wrapper: VueWrapper | null = null

function lessonsPlatform(): FakePlatform {
  const platform = fakePlatform()
  platform.listLessons.mockResolvedValue(LESSONS)
  return platform
}

async function mountLessons(platform: FakePlatform): Promise<VueWrapper> {
  wrapper = mountWith(LessonsView, { platform })
  await flushPromises()
  return wrapper
}

const find = (selector: string) => wrapper!.find(selector)

async function click(selector: string): Promise<void> {
  await find(selector).trigger('click')
  await flushPromises()
}

afterEach(() => {
  wrapper?.unmount()
  wrapper = null
})

describe('the lesson list', () => {
  it('titles lessons from their languages and their number within the pair', async () => {
    await mountLessons(lessonsPlatform())
    const titles = () => wrapper!.findAll('[data-test="lesson-title"]').map((node) => node.text())
    expect(titles()).toEqual([
      'English → German · Lesson 1',
      'Chinese → English · Lesson 1',
      'English → German · Lesson 2',
    ])
    expect(wrapper!.text()).toContain('Words: 2')

    setLocale('de')
    await flushPromises()
    expect(titles()).toEqual([
      'Englisch → Deutsch · Lektion 1',
      'Chinesisch → Englisch · Lektion 1',
      'Englisch → Deutsch · Lektion 2',
    ])
  })

  it('explains where lessons come from when there are none', async () => {
    await mountLessons(fakePlatform())
    expect(find('[data-test="lessons-empty"]').text()).toContain('example sentence')
  })

  it('shows why the lessons could not be loaded', async () => {
    const platform = fakePlatform()
    platform.listLessons.mockRejectedValueOnce('database is locked')
    await mountLessons(platform)
    expect(find('[data-test="lessons-error"]').text()).toContain('database is locked')
  })
})

describe('a lesson', () => {
  it('shows each word in context, then its sentence as a cloze to reveal', async () => {
    await mountLessons(lessonsPlatform())
    await click('[data-test="open-lesson-11"]')
    expect(find('[data-test="active-lesson-title"]').text()).toBe('English → German · Lesson 1')
    expect(find('[data-test="progress"]').attributes('aria-valuetext')).toBe('Step 1 of 4')

    const context = find('[data-test="step-context"]')
    expect(context.find('[data-test="word"]').text()).toBe('serendipity')
    expect(context.text()).toContain('glücklicher Zufall')
    expect(context.find('[data-test="sentence-source"]').text()).toBe(
      'Finding it was pure serendipity.',
    )
    expect(context.find('[data-test="sentence-target"]').text()).toBe(
      'Es zu finden war reiner Zufall.',
    )

    await click('[data-test="next"]')
    const cloze = find('[data-test="cloze"]')
    expect(cloze.text()).toBe('Finding it was pure .')
    expect(cloze.find('[role="img"]').attributes('aria-label')).toBe('gap')
    expect(find('[data-test="step-cloze"]').text()).toContain('Es zu finden war reiner Zufall.')
    expect(find('[data-test="cloze-word"]').exists()).toBe(false)

    await click('[data-test="reveal"]')
    expect(find('[data-test="cloze-word"]').text()).toBe('serendipity')
    expect(find('[data-test="cloze"]').text()).toBe('Finding it was pure serendipity.')

    await click('[data-test="next"]')
    expect(find('[data-test="word"]').text()).toBe('cat')
    await click('[data-test="previous"]')
    expect(find('[data-test="cloze-word"]').exists()).toBe(false)
  })

  it('completes once at the end and shows the XP', async () => {
    const platform = lessonsPlatform()
    await mountLessons(platform)
    await click('[data-test="open-lesson-11"]')
    expect(find('[data-test="complete"]').exists()).toBe(false)
    for (let step = 0; step < 3; step += 1) await click('[data-test="next"]')
    expect(find('[data-test="next"]').exists()).toBe(false)

    await click('[data-test="complete"]')
    await click('[data-test="complete"]')
    expect(platform.completeLesson).toHaveBeenCalledTimes(1)
    expect(platform.completeLesson).toHaveBeenCalledWith('lesson-11')
    expect(find('[data-test="lesson-result"]').text()).toBe('Lesson complete! +20 XP')
  })

  it('says when today’s XP for the lesson was earned already', async () => {
    const platform = lessonsPlatform()
    platform.completeLesson.mockResolvedValue(0)
    await mountLessons(platform)
    await click('[data-test="open-lesson-20"]')
    await click('[data-test="next"]')
    await click('[data-test="complete"]')
    expect(find('[data-test="lesson-result"]').text()).toContain('already earned XP')
  })

  it('reports a lesson that could not be completed, and can try again', async () => {
    const platform = lessonsPlatform()
    platform.completeLesson.mockRejectedValueOnce('database is locked')
    await mountLessons(platform)
    await click('[data-test="open-lesson-20"]')
    await click('[data-test="next"]')
    await click('[data-test="complete"]')
    expect(find('[data-test="complete-error"]').text()).toContain('database is locked')
    await click('[data-test="complete"]')
    expect(platform.completeLesson).toHaveBeenCalledTimes(2)
    expect(find('[data-test="lesson-result"]').text()).toContain('+20 XP')
  })

  it('marks hanzi and pinyin for a Chinese lesson and returns to the list', async () => {
    await mountLessons(lessonsPlatform())
    await click('[data-test="open-lesson-20"]')
    expect(find('[data-test="word"]').attributes('lang')).toBe('zh-CN')
    expect(find('[data-test="step-context"]').text()).toContain('xué xí')
    await click('[data-test="next"]')
    expect(find('[data-test="cloze"]').text()).toBe('我在中文。')
    await click('[data-test="close"]')
    expect(wrapper!.findAll('[data-test="lesson"]')).toHaveLength(3)
  })
})
