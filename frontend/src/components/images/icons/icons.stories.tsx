import type { Meta } from '@storybook/react';
import { WifiSignalStrength, WidgetPool, WidgetManager, WidgetClocks, WidgetCombined, WidgetTicker } from './index';

export default {
    title: 'components/Icons',
} satisfies Meta;

const display = 'flex';
const padding = 8;
const gap = 16;

function Column(props: { children: ReactNode }) {
    return <div style={{ display, flexDirection: 'column', gap, padding, width: 600 }} children={props.children} />;
}
function Row(props: { children: ReactNode }) {
    return <div style={{ display, flexDirection: 'row', gap, padding }} children={props.children} />;
}

export function Icons() {
    return (
        <Column>
            <Row>
                <WifiSignalStrength size={64} state="full" />
                <WifiSignalStrength size={64} state="fair" />
                <WifiSignalStrength size={64} state="low" />
                <WifiSignalStrength size={64} state="offline" />
                <WifiSignalStrength size={64} state="scanning" />
            </Row>
            <Row>
                {[WidgetPool, WidgetManager, WidgetClocks, WidgetCombined, WidgetTicker].map((Icon, i) => (
                    <Icon key={i} size={64} />
                ))}
            </Row>
        </Column>
    );
}
