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

import type { Meta } from '@storybook/react';
import { action } from 'storybook/actions';
import styled from '@emotion/styled';

import * as pb from '@/proto';
import * as X from './index';

export default {
    title: 'accounts/Forms',
} satisfies Meta;

const Wrapper = styled.div`
    display: inline flex;
    flex-flow: column nowrap;
    background-color: var(--cds-layer-01);
    min-inline-size: 544px;
    padding: 1rem;
`;
const BRAIINS_FORM_ARGS: X.FormBraiinsPoolProps = {
    name: {
        value: '',
        onChange: action('onChange'),
    },
    apiKey: {
        value: '',
        onChange: action('onChange'),
    },
};

export function FormCombined(args: X.FormCombinedProps) {
    return (
        <Wrapper>
            <X.FormCombined {...args} />
        </Wrapper>
    );
}
FormCombined.storyName = 'Combined';
FormCombined.args = {
    type: {
        value: pb.AccountType.BRAIINSPOOL,
        onChange: action('onChange'),
    },
    valuesBraiinsPool: BRAIINS_FORM_ARGS,
    connectedWidgetsCount: null,
} satisfies X.FormCombinedProps;

export function FormBraiinsPool(args: X.FormBraiinsPoolProps) {
    return (
        <Wrapper>
            <X.FormBraiinsPool {...args} />
        </Wrapper>
    );
}
FormBraiinsPool.storyName = 'BraiinsPool';
FormBraiinsPool.args = BRAIINS_FORM_ARGS;
