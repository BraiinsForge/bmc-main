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

// biome-ignore lint/style/noRestrictedImports: the component this wraps
import { ProgressStep as Upstream, type ProgressStepProps } from '@carbon/react';
import { assertUnreachable } from '@/lib/ts';

type TranslationKey = Parameters<NonNullable<ProgressStepProps['translateWithId']>>[0];

export function ProgressStep(props: ProgressStepProps) {
    const { formatMessage } = useIntl();
    const translateWithId = (id: TranslationKey): string => {
        switch (id) {
            case 'carbon.progress-step.complete':
                return formatMessage({
                    defaultMessage: 'Complete',
                    description: "Screen-reader state read after a progress step's label: the step is done",
                });
            case 'carbon.progress-step.current':
                return formatMessage({
                    defaultMessage: 'Current',
                    description:
                        "Screen-reader state read after a progress step's label: the step is the one in progress",
                });
            case 'carbon.progress-step.incomplete':
                return formatMessage({
                    defaultMessage: 'Incomplete',
                    description: "Screen-reader state read after a progress step's label: the step is still to come",
                });
            case 'carbon.progress-step.invalid':
                return formatMessage({
                    defaultMessage: 'Invalid',
                    description: "Screen-reader state read after a progress step's label: the step needs fixing",
                });
            default:
                return assertUnreachable(id, 'progress step translation id');
        }
    };
    return <Upstream translateWithId={translateWithId} {...props} />;
}
