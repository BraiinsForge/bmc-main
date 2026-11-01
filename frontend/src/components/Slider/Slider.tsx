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

import type { ComponentProps } from 'react';
import { useIntl } from 'react-intl';

// biome-ignore lint/style/noRestrictedImports: the component this wraps
import { Slider as Upstream } from '@carbon/react';
import { assertUnreachable } from '@/lib/ts';

export type SliderProps = ComponentProps<typeof Upstream>;
type TranslateWithId = NonNullable<SliderProps['translateWithId']>;

export function Slider(props: SliderProps) {
    const { formatMessage } = useIntl();

    const translateWithId: TranslateWithId = (id, args) => {
        switch (id) {
            case 'carbon.slider.auto-correct-announcement':
                return formatMessage(
                    {
                        defaultMessage: 'The value “{correctedValue}” was corrected to the nearest allowed value.',
                        description:
                            "Alert under a slider's number field after it moved a typed value into its range; correctedValue is the value typed, such as 150",
                    },
                    { correctedValue: args?.correctedValue ?? '' },
                );

            default:
                return assertUnreachable(id, 'slider translation id');
        }
    };

    return <Upstream translateWithId={translateWithId} {...props} />;
}
