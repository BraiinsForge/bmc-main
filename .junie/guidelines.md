# Project Guidelines

- the "frontend" directory contains everything related to frontend

## Frontend

- never import or use React namespace, it is implicit
  - examples of what shall never occur are `import React from 'react'` or `React.Component`
  - all react types shall be explicitly imported and include `type` import preffix (except when used as value)
  - state and props are always defined as interfaces
  - props are named the same as the component plus `Props` suffix
    - example: `interface ComponentNameProps {}`
  - if there is a state, it's type declaration is followed by `getInitialState` function
  - Do not use curly braces where un-neccessary

### Tests

- always create separate file named after the module that it tests with .spec suffix added
  - examples:
    - module.ts → module.spec.ts
    - module.tsx → module.spec.tsx
- always import test functions like "describe", "test", "expec" from "@rstest/core"
- do not use "it", use "test"
