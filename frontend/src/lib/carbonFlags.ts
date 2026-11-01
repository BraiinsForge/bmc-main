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

import { enable } from '@carbon/feature-flags';

// Carbon's modals, notifications and floating menus otherwise trap focus between
// hidden `role="link"` spans that a screen reader reads out as "Focus sentinel", in English.
// The global flags, not a <FeatureFlags> provider: Carbon 1.119's FloatingMenu reads only
// the globals, and the context the others read falls back to them when no provider is mounted.
enable('enable-focus-wrap-without-sentinels');
