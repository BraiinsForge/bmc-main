// Copyright (C) 2026  Braiins Systems s.r.o.
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

import { Helmet } from '@dr.pogodin/react-helmet';

import { useStore } from '@/store';

const DECK_NAME = 'Braiins DECK';

/** Tab title: the Deck's own, or the managing boser's brand name on a BMM. `boser-tab.js` swaps the icon. */
export function AppHead() {
    const name = useStore(x => x.state.brand?.name) ?? DECK_NAME;
    return <Helmet defaultTitle={name} titleTemplate={`%s | ${name}`} />;
}
