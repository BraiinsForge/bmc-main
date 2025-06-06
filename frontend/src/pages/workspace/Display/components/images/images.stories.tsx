import styled from '@emotion/styled';
import type { Meta } from '@storybook/react';
import type { ReactNode, ReactElement } from 'react';

import * as pb from '@/proto';
import * as preview from './preview';

export default {
    title: 'display/components/ScenePreview',
} satisfies Meta;

const Column = styled.div`
    display: flex;
    flex-flow: column nowrap;
    gap: 16px;
`;
function Row(props: { title: string; children: ReactNode }) {
    const { title, children } = props;
    const Root = styled.div`
        display: flex;
        flex-direction: column;
        gap: 2px;
    `;
    const Header = styled.header`
        padding: 8px;
        background: var(--cds-layer-01);
    `;
    const Main = styled.div`
        display: flex;
        flex-direction: row;
        gap: 2px;
    `;

    return (
        <Root>
            <Header children={title} />
            <Main children={children} />
        </Root>
    );
}

const Header = styled.header`
    display: flex;
    background: var(--cds-layer-01);
    padding: 8px 12px;
    font-family: monospace;
`;
function Cell(props: { title: string; children: ReactNode }): ReactElement {
    const { title, children } = props;

    return (
        <div style={{ display: 'flex', flexDirection: 'column', gap: 2 }}>
            <style scoped children="img { max-inline-size: 300px }" />
            <Header children={title} />
            <main children={children} />
        </div>
    );
}

export function ScenePreview() {
    return (
        <Column>
            <Row title="clock">
                <Cell title="analog-rect">
                    <preview.ScenePreview kind={pb.SceneKind.clock} variant={pb.SceneVariantClock.analog_rect} />
                </Cell>
                <Cell title="analog-round">
                    <preview.ScenePreview kind={pb.SceneKind.clock} variant={pb.SceneVariantClock.analog_round} />
                </Cell>
                <Cell title="digital-flip">
                    <preview.ScenePreview kind={pb.SceneKind.clock} variant={pb.SceneVariantClock.digital_flip} />
                </Cell>
                <Cell title="digital-plain">
                    <preview.ScenePreview kind={pb.SceneKind.clock} variant={pb.SceneVariantClock.digital_plain} />
                </Cell>
            </Row>

            <Row title="ticker">
                <Cell title="line">
                    <preview.ScenePreview kind={pb.SceneKind.ticker} variant={pb.SceneVariantTicker.line} />
                </Cell>
                <Cell title="list">
                    <preview.ScenePreview kind={pb.SceneKind.ticker} variant={pb.SceneVariantTicker.list} />
                </Cell>
                <Cell title="candle">
                    <preview.ScenePreview kind={pb.SceneKind.ticker} variant={pb.SceneVariantTicker.candle} />
                </Cell>
            </Row>

            <Row title="image">
                <preview.ScenePreview kind={pb.SceneKind.image} />
            </Row>

            <Row title="pool">
                <preview.ScenePreview kind={pb.SceneKind.pool} />
            </Row>

            <Row title="manager">
                <preview.ScenePreview kind={pb.SceneKind.manager} />
            </Row>

            <Row title="combined">
                <preview.ScenePreview kind={pb.SceneKind.combined} />
            </Row>
        </Column>
    );
}
