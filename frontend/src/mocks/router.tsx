// Copyright (C) 2026  Braiins Forge s.r.o.
//
// This program is free software: you can redistribute it and/or modify
// it under the terms of the GNU General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.
//
// This program is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
// GNU General Public License for more details.
//
// You should have received a copy of the GNU General Public License
// along with this program.  If not, see <https://www.gnu.org/licenses/>.
//
// Braiins Systems s.r.o. and Braiins Forge s.r.o. each reserve the right
// to grant any party a license to this program, or any part thereof,
// under any terms, and such a grant shall be considered distinct from
// the grant above.

import type { ReactNode } from 'react';
import { MemoryRouter } from 'react-router';

/// Router context for a subtree under test: `useNavigate` throws without one,
/// and the house `Link` calls it on every render.
///
/// Deliberately no router-*prop* mock beside it: this app reaches for the hooks,
/// and a prop mock belongs with a `withRouter` HOC that this app does not have.
export const withRouter = (ui: ReactNode) => <MemoryRouter>{ui}</MemoryRouter>;
