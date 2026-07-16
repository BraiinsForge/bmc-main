// Copyright (C) 2025  Braiins Systems s.r.o.
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

import { LayoutPlain as Component } from './LayoutPlain';

export default {
    title: 'layouts/LayoutPlain',
    component: Component,
};

export function LayoutPlain() {
    return (
        <Component>
            Lorem ipsum dolor sit amet, consectetur adipisicing elit. Assumenda atque, consequatur cumque dolores
            dolorum in minima molestiae natus, officiis, omnis pariatur quisquam tempore ullam voluptate voluptatem.
            Aliquid dignissimos eaque eveniet?
        </Component>
    );
}
