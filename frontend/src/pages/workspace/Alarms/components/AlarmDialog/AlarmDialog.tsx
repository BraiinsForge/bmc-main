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

import type { ReactElement } from 'react';
import { useIntl } from 'react-intl';

import { getID } from '@/lib/form';
import type * as pb from '@/proto';

import { TrashCan as IconDelete } from '@carbon/react/icons';
import { Button, Modal, InlineNotificationsGroup } from '@/components';
import { AlarmForm, type AlarmFormProps } from '../AlarmForm';

import css from './AlarmDialog.scss';

export interface AlarmDialogProps {
    open: boolean;
    // `null` while adding one
    alarmID: null | pb.Alarm['id'];
    errors: Maybe<string[]>;
    form: AlarmFormProps;
    onSubmit(): void;
    onClose(): void;
    onDelete(id: pb.Alarm['id']): void;
}

const $ = getID('alarms').get;

export function AlarmDialog({
    open,
    alarmID,
    errors,
    form,
    onSubmit,
    onClose,
    onDelete,
}: AlarmDialogProps): ReactElement {
    const { formatMessage } = useIntl();
    const editing = alarmID != null;

    return (
        <Modal
            id={$('add-dialog')}
            open={open}
            size="sm"
            // Labeling & behavior
            selectorPrimaryFocus="input"
            modalHeading={
                editing
                    ? formatMessage({
                          defaultMessage: 'Edit Alarm',
                          description: 'Alarm dialog title while changing an existing alarm',
                      })
                    : formatMessage({
                          defaultMessage: 'Add New Alarm',
                          description: 'Alarm dialog title while creating an alarm',
                      })
            }
            // Submit
            primaryButtonText={
                editing
                    ? formatMessage({
                          defaultMessage: 'Confirm Changes',
                          description: 'Alarm dialog button that saves the changes to an existing alarm',
                      })
                    : formatMessage({
                          defaultMessage: 'Add New Alarm',
                          description: 'Alarm dialog button that creates the alarm',
                      })
            }
            onRequestSubmit={onSubmit}
            // Cancel
            onRequestClose={onClose}
            secondaryButtonText={formatMessage({ defaultMessage: 'Cancel' })}
            onSecondarySubmit={onClose}
        >
            <InlineNotificationsGroup items={errors} kind="error" stretch className={css.errors} />

            <AlarmForm {...form} />

            {editing ? (
                <div className={css.deleteButtonRow}>
                    <Button
                        id={$('delete-alarm', alarmID)}
                        kind="danger"
                        icon={IconDelete}
                        children={formatMessage({
                            defaultMessage: 'Delete Alarm',
                            description: 'Alarm dialog button that deletes the alarm being changed',
                        })}
                        onClick={() => onDelete(alarmID)}
                    />
                </div>
            ) : null}
        </Modal>
    );
}
