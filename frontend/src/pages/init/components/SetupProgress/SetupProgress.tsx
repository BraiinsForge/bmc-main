// Copyright (C) 2026  Braiins Systems s.r.o.
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

import { useIntl } from 'react-intl';

// Components
import { ProgressStep } from '@/components';
import { ProgressIndicator } from '@carbon/react';

// Styles
import css from './SetupProgress.scss';

const STEP_INDEX = { wifi: 0, device: 1 } as const;

export function SetupProgress(props: { step: keyof typeof STEP_INDEX }) {
    const { formatMessage } = useIntl();

    return (
        <ProgressIndicator currentIndex={STEP_INDEX[props.step]} className={css.progress}>
            <ProgressStep
                label={formatMessage({
                    defaultMessage: 'Wi-Fi Settings',
                    description: 'First step of the progress at the top of the setup pages: connecting to Wi-Fi',
                })}
            />
            <ProgressStep
                label={formatMessage({
                    defaultMessage: 'Initial Setup',
                    description: 'Second step of the progress at the top of the setup pages: setting up the device',
                })}
                className={css.disabledTab}
            />
        </ProgressIndicator>
    );
}
