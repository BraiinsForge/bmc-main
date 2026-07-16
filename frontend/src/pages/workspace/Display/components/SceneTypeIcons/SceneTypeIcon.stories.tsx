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

import { SceneTypeIcons as Component, type SceneTypeIconsProps } from './SceneTypeIcons';
import styled from '@emotion/styled';

export default {
    title: 'Display/Components/SceneTypeIcon',
    component: Component,
};

const cases: SceneTypeIconsProps[] = [{ night: true }];

const Wrapper = styled.div`
    display: flex;
    flex-direction: column;
    align-items: stretch;
    gap: 16px;
`;
const Row = styled.div`
    display: inline-flex;
    flex-flow: column;
    padding: 16px;
    gap: 8px;
    background-color: var(--cds-layer-01);
`;
const Code = styled.pre`
    display: inline-block;
`;

export function SceneTypeIcon() {
    return (
        <Wrapper
            children={cases.map((props, i) => {
                return (
                    <Row key={i}>
                        <Code
                            children={Object.entries(props)
                                .filter(([_, v]) => typeof v === 'boolean')
                                .map(([k, _]) => `${k}`)
                                .join(', ')}
                        />
                        <Component {...props} />
                    </Row>
                );
            })}
        />
    );
}
