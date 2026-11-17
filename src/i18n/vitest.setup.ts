// Vitest `setupFiles` entry (`vitest.config.ts`): runs before every test file, in every environment (jsdom and
//   `// @vitest-environment node`). Frontend tests assert Korean text, so they run in Korean; a test that needs English sets
//   it and restores Korean. In jsdom this also sets `<html lang="ko">`, so a module re-imported after `vi.resetModules()`
//   boots in Korean too.

import { setLanguage } from './index'

setLanguage('ko')
