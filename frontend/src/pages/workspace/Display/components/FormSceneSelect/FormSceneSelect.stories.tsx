import { action } from 'storybook/actions';
import type { Meta } from '@storybook/react';

import * as pb from '@/proto';
import { FormSceneSelect as Component, type FormSceneSelectProps } from './FormSceneSelect';

export default {
    title: 'Display/Components/FormSceneSelect',
    component: Component,
} satisfies Meta<FormSceneSelectProps>;

function manifest(
    uid: string,
    name: string,
    description: string,
    category: pb.WidgetCategory,
    subname?: string,
): pb.WidgetManifest {
    return pb.create(pb.WidgetManifestSchema, {
        uid,
        name,
        subname,
        description,
        category,
        version: '1.0.0',
        supportedSizes: [pb.WidgetSize.FULL],
        params: [],
    });
}

const C = pb.WidgetCategory;

// Spread across categories (and deliberately out of section order) to exercise
// the category grouping, the `misc`-last ordering, and the within-section name
// sort. Also mixes widgets with and without a `subname`, including a long one
// that wraps to a second line.
const manifestWidgets: pb.WidgetManifest[] = [
    manifest('clock', 'Clock', 'Displays the current time.', C.CLOCK, 'Digital'),
    manifest('ticker', 'Ticker', 'Live BTC/USD price.', C.MISC, 'BTC/USD'),
    manifest('block-height', 'Block Height', 'Current Bitcoin block height.', C.MINING, 'Mainnet chain tip'),
    manifest('halving', 'Halving Countdown', 'Time until the next halving.', C.MINING),
    manifest('weather', 'Weather', 'Local weather and forecast.', C.WEATHER, 'Forecast'),
    manifest('iss', 'ISS Position', 'Where the station is right now.', C.SPACE),
    manifest('params', 'Params Demo', 'Read-back exemplar for every ParamKind.', C.UTILITY, 'Param showcase'),
    manifest('nameday', 'Nameday', "Today's nameday celebrations.", C.CALENDAR),
    manifest('random-facts', 'Random Facts', 'Display a random factoid.', C.KNOWLEDGE),
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
