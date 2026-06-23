import { action } from 'storybook/actions';
import type { Meta } from '@storybook/react';

import * as pb from '@/proto';
import { FormSceneSelect as Component, type FormSceneSelectProps } from './FormSceneSelect';

export default {
    title: 'Display/Components/FormSceneSelect',
    component: Component,
} satisfies Meta<FormSceneSelectProps>;

function manifest(uid: string, name: string, description: string, subname?: string): pb.WidgetManifest {
    return pb.create(pb.WidgetManifestSchema, {
        uid,
        name,
        subname,
        description,
        version: '1.0.0',
        supportedSizes: [pb.WidgetSize.FULL],
        params: [],
    });
}

// Mix of widgets with and without a `subname`, including a long one that wraps
// to a second line — exercises the grayed inline label and the cell growth.
const manifestWidgets: pb.WidgetManifest[] = [
    manifest('clock', 'Clock', 'Displays the current time.', 'Digital'),
    manifest('ticker', 'Ticker', 'Live BTC/USD price.', 'BTC/USD'),
    manifest('block-height', 'Block Height', 'Current Bitcoin block height.', 'Mainnet chain tip'),
    manifest('halving', 'Halving Countdown', 'Time until the next halving.'),
    manifest('weather', 'Weather', 'Local weather and forecast.', 'Forecast'),
    manifest('nameday', 'Nameday', "Today's nameday celebrations."),
];

export const Populated = () => (
    <Component
        isOpen
        onClose={action('onClose')}
        onManifestSelection={action('onManifestSelection')}
        manifestWidgets={manifestWidgets}
        isLoading={false}
    />
);

export const Loading = () => (
    <Component
        isOpen
        onClose={action('onClose')}
        onManifestSelection={action('onManifestSelection')}
        manifestWidgets={[]}
        isLoading
    />
);

export const Empty = () => (
    <Component
        isOpen
        onClose={action('onClose')}
        onManifestSelection={action('onManifestSelection')}
        manifestWidgets={[]}
        isLoading={false}
    />
);
