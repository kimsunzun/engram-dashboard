// Test fixture for index.test.ts: a `t()` call at module scope, evaluated during import like the command titles. It shows
//   the backend is ready when a module that imports `t` is evaluated (synchronous init — ADR-0373).

import { t } from './index'

export const MODULE_SCOPE_TITLE = t('tab.create')
