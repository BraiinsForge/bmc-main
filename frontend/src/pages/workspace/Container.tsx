// Copyright (C) 2025  Braiins Systems s.r.o.
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

import { Outlet } from 'react-router';
import { LayoutWorkspace } from '@/layouts/LayoutWorkspace';
import { useStore } from '@/store';

export default function WorkspaceContainer() {
    const isAuthenticated = useStore(x => x.state.sessionInfo.isAuthenticated);

    // A page mounts only if the last session check said yes,
    // so a visit without a session loads nothing the device would refuse.
    const content = isAuthenticated === true ? <Outlet /> : null;

    return <LayoutWorkspace children={content} />;
}
