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
