import { flushPromises } from '@vue/test-utils'
import { effectScope, nextTick, ref } from 'vue'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

import { callsTo, deferred, onCommand, resetTauri } from '../test/fakeTauri'
import { createModelFields, type ModelFields } from './useModelFields'

vi.mock('@tauri-apps/api/core', async () => (await import('../test/fakeTauri')).coreModule)

let scope: ReturnType<typeof effectScope> | null = null
const online = ref(true)

function cache(): ModelFields {
  scope = effectScope()
  const created = scope.run(() => createModelFields(() => online.value))
  if (!created) throw new Error('no cache')
  return created
}

beforeEach(() => {
  resetTauri()
  online.value = true
})

afterEach(() => {
  scope?.stop()
  scope = null
})

describe('createModelFields', () => {
  it('reads each note type once and shares a request in flight', async () => {
    const answer = deferred<string[]>()
    onCommand('anki_model_field_names', () => answer.promise)
    const fields = cache()
    fields.load('AppleStyleFinalV3')
    fields.load(' AppleStyleFinalV3 ')
    expect(fields.state('AppleStyleFinalV3')).toEqual({ fields: null, loading: true, error: null })

    answer.resolve(['Front', 'Back'])
    await flushPromises()
    fields.load('AppleStyleFinalV3')
    expect(fields.state('AppleStyleFinalV3')).toEqual({
      fields: ['Front', 'Back'],
      loading: false,
      error: null,
    })
    expect(callsTo('anki_model_field_names').map((call) => call.args)).toEqual([
      { model: 'AppleStyleFinalV3' },
    ])
  })

  it('reads nothing offline or without a name', () => {
    onCommand('anki_model_field_names', ['Front'])
    const fields = cache()
    fields.load('  ')
    online.value = false
    fields.load('Basic')
    expect(callsTo('anki_model_field_names')).toHaveLength(0)
    expect(fields.state('Basic')).toEqual({ fields: null, loading: false, error: null })
  })

  it('keeps a failed read until it is asked for again', async () => {
    onCommand('anki_model_field_names', () => {
      throw 'model was not found'
    })
    const fields = cache()
    fields.load('Gone')
    await flushPromises()
    expect(fields.state('Gone')).toEqual({
      fields: null,
      loading: false,
      error: 'model was not found',
    })
    onCommand('anki_model_field_names', ['Front'])
    fields.load('Gone')
    await flushPromises()
    expect(fields.state('Gone').fields).toEqual(['Front'])
  })

  it('forgets everything when Anki goes offline, including answers still on their way', async () => {
    const late = deferred<string[]>()
    onCommand('anki_model_field_names', ({ model }: { model: string }) =>
      model === 'Late' ? late.promise : ['Front'],
    )
    const fields = cache()
    fields.load('Basic')
    fields.load('Late')
    await flushPromises()
    expect(fields.state('Basic').fields).toEqual(['Front'])

    online.value = false
    await nextTick()
    late.resolve(['Old'])
    await flushPromises()
    expect(fields.state('Basic').fields).toBeNull()
    expect(fields.state('Late')).toEqual({ fields: null, loading: false, error: null })

    online.value = true
    fields.load('Basic')
    await flushPromises()
    expect(callsTo('anki_model_field_names')).toHaveLength(3)
  })
})
