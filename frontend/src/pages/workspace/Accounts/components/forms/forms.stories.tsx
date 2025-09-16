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
