# Project Guidelines

- the "frontend" directory contains everything related to frontend

## Frontend

- never import or use React namespace, it is implicit
  - examples of what shall never occur are `import React from 'react'` or `React.Component`
  - all react types shall be explicitly imported and include `type` import preffix (except when used as value)
  - state and props are always defined as interfaces
  - props are named the same as the component plus `Props` suffix
  - if there is a state, it's type declaration is followed by `getInitialState` function
