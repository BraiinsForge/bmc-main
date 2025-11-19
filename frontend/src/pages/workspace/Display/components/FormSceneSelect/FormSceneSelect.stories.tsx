import type { Meta } from '@storybook/react';
import { action } from 'storybook/actions';
import * as pb from '@/proto';
import { FormSceneSelect as Component, type FormSceneSelectProps } from './FormSceneSelect';

export default {
    title: 'display/components/FormSceneSelect',
    component: Component,
} satisfies Meta<FormSceneSelectProps>;

const remoteWidgetRecents: pb.RemoteWidget[] = [
    pb.create(pb.RemoteWidgetSchema, {
        name: 'Exchange Rate',
        description: 'Display real-time currency exchange rates and conversion information',
        widgetUrl: 'https://braiinsforge.com/deck/widgets/exchange-rate',
        iconUrl: 'https://braiinsforge.com/deck/widgets/exchange-rate/icon?rev=ac6224936b035e71',
    }),
    pb.create(pb.RemoteWidgetSchema, {
        name: 'Nameday',
        description: "Shows today's nameday celebrations and upcoming name days in your region",
        widgetUrl: 'https://braiinsforge.com/deck/widgets/nameday',
        iconUrl: 'https://braiinsforge.com/deck/widgets/nameday/icon?rev=baf9fcc58f8e24b8',
    }),
    pb.create(pb.RemoteWidgetSchema, {
        name: 'Financial Ticker List',
        description: 'Track multiple stocks, crypto, and commodities with live price updates and performance metrics',
        widgetUrl: 'https://braiinsforge.com/deck/widgets/ticker-list',
        iconUrl: 'https://braiinsforge.com/deck/widgets/ticker-list/icon?rev=265c6ed448a7c013',
    }),
    pb.create(pb.RemoteWidgetSchema, {
        name: 'Ticker - Single Candlestick',
        description:
            'Visualize price movements with detailed candlestick chart showing open, high, low, and close values',
        widgetUrl: 'https://braiinsforge.com/deck/widgets/ticker-single-candlestick',
        iconUrl: 'https://braiinsforge.com/deck/widgets/ticker-single-candlestick/icon?rev=37e9e9b0177a093d',
    }),
    pb.create(pb.RemoteWidgetSchema, {
        name: 'Ticker - Single Sparkline',
        description: 'Compact price trend visualization with minimalist sparkline chart for quick insights',
        widgetUrl: 'https://braiinsforge.com/deck/widgets/ticker-single-sparkline',
        iconUrl: 'https://braiinsforge.com/deck/widgets/ticker-single-sparkline/icon?rev=31e233defb663446',
    }),
    pb.create(pb.RemoteWidgetSchema, {
        name: 'Weather',
        description: 'Current conditions and forecast with temperature, precipitation, and wind information',
        widgetUrl: 'https://braiinsforge.com/deck/widgets/weather',
        iconUrl: 'https://braiinsforge.com/deck/widgets/weather/icon?rev=1f494d0a8e5c98d7',
    }),
];

export function FormSceneSelect() {
    return (
        <Component
            onSelection={action('onClick')}
            onClose={action('onClose')}
            isOpen={true}
            remoteWidgetUrl={{
                value: '',
                error: null,
                disabled: false,
                onChange: action('remoteWidgetUrl.onChange'),
                onSubmit: action('remoteWidgetUrl.onSubmit'),
            }}
            remoteWidgetRecents={remoteWidgetRecents}
        />
    );
}
