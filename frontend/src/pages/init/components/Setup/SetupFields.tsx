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

import type { KeyboardEvent, ReactElement } from 'react';
import { useIntl } from 'react-intl';
import { Key } from 'ts-key-enum';

import * as pb from '@/proto';
import { blockEvent } from '@/lib/react';
import type { iField } from '@/lib/form';

// Components
import { ButtonSwitch, type ButtonSwitchItem, FieldSet, Field, Tooltip } from '@/components';
import { ComboBox, type ComboBoxProps, Dropdown, type DropdownProps, PasswordInput } from '@carbon/react';
import { Information as IconInfo } from '@carbon/react/icons';

// Styles
import css from './Setup.scss';

export type TimeFormat = Exclude<pb.TimeFormat, 0>;
export type DateFormat = Exclude<pb.DateFormat, 0>;
export type NumberFormat = Exclude<pb.NumberFormat, 0>;
export type TemperatureUnit = Exclude<pb.TemperatureUnit, 0>;
export type UnitSystem = Exclude<pb.UnitSystem, 0>;

/** Element id builder of the form the fields belong to. */
export type IdOf = (name: string) => string;

/** Blur a focused input on Escape instead of letting the key bubble. */
export function catchEscapeKey(e: KeyboardEvent<HTMLFormElement>): void {
    if (e.target instanceof HTMLInputElement && e.key === Key.Escape) {
        blockEvent(e);
        e.target.blur();
    }
}

export interface LocalizationFieldsProps {
    timeFormat: iField<TimeFormat>;
    timezone: iField<pb.Timezone> & { items: ReadonlyArray<pb.Timezone> };
    dateFormat: iField<DateFormat>;
    numberFormat: iField<NumberFormat>;
    temperatureUnits: iField<TemperatureUnit>;
    unitSystem: iField<UnitSystem>;
}

function renderTimezone(tz: pb.Timezone): ReactElement {
    return (
        <span className={css.timezoneElement}>
            <span children={`UTC${tz.offset}`} className={css.mono} />
            <span children={`(${tz.label})`} />
        </span>
    );
}

/** The "Time, Date and Regional Settings" fieldset every device sets up. */
export function LocalizationFields(props: LocalizationFieldsProps & { $: IdOf }) {
    const { $, timeFormat, timezone, dateFormat, numberFormat, temperatureUnits, unitSystem } = props;
    const intl = useIntl();
    const { formatMessage } = intl;

    const onTimezoneChange: ComboBoxProps<pb.Timezone>['onChange'] = x => {
        // Only update if a new item is selected, prevent clearing on ESC
        if (x.selectedItem) timezone.onChange?.(x.selectedItem);
        // ESC was pressed - restore the current value
        else if (x.selectedItem == null && timezone.value) timezone.onChange?.(timezone.value);
    };
    const onDateFormatChange: DropdownProps<DateFormat>['onChange'] = x => {
        if (x.selectedItem) dateFormat.onChange?.(x.selectedItem);
    };
    const onNumberFormatChange: DropdownProps<NumberFormat>['onChange'] = x => {
        if (x.selectedItem) numberFormat.onChange?.(x.selectedItem);
    };

    const temperatureOptions = Array.from(pb.temperatureUnitOptions.entries()).map<ButtonSwitchItem<TemperatureUnit>>(
        ([key, Icon]) => ({
            id: key,
            text: pb.temperatureUnitToString(intl, key) ?? 'N/A',
            icon: Icon,
        }),
    );
    const unitSystemOptions = pb.unitSystemOptions.map<ButtonSwitchItem<UnitSystem>>(key => ({
        id: key,
        text: pb.unitSystemToString(intl, key) ?? 'N/A',
    }));

    return (
        <FieldSet title={formatMessage({ defaultMessage: 'Time, Date and Regional Settings' })}>
            <Field
                variant="light"
                title={formatMessage({ defaultMessage: 'Time Format' })}
                disabled={timeFormat.disabled}
            >
                <ButtonSwitch<TimeFormat>
                    id={$('time-format')}
                    selectedOption={timeFormat.value}
                    options={[
                        {
                            id: pb.TimeFormat.TIME_FORMAT_12_HOUR,
                            text: formatMessage({ defaultMessage: '12-Hour' }),
                        },
                        {
                            id: pb.TimeFormat.TIME_FORMAT_24_HOUR,
                            text: formatMessage({ defaultMessage: '24-Hour' }),
                        },
                    ]}
                    onChange={timeFormat.onChange}
                    disabled={timeFormat.disabled}
                    invalid={!!timeFormat.error}
                    invalidText={timeFormat.error}
                />
            </Field>

            <Field variant="light" title={formatMessage({ defaultMessage: 'Timezone' })} disabled={timezone.disabled}>
                <ComboBox<pb.Timezone>
                    id={$('timezone')}
                    titleText=""
                    disabled={timezone.disabled}
                    direction="bottom"
                    items={Array.from(timezone.items)}
                    onChange={onTimezoneChange}
                    itemToString={pb.renderTimezone}
                    itemToElement={renderTimezone}
                    selectedItem={timezone.value}
                    invalid={!!timezone.error}
                    invalidText={timezone.error}
                    className={css.timezoneComboBox}
                />
            </Field>

            <Field
                variant="light"
                title={formatMessage({ defaultMessage: 'Date Format' })}
                disabled={dateFormat.disabled}
            >
                <Dropdown<DateFormat>
                    id={$('date-format')}
                    size="md"
                    label=""
                    titleText=""
                    hideLabel
                    items={pb.dateFormatOptions}
                    selectedItem={dateFormat.value ?? undefined}
                    onChange={onDateFormatChange}
                    itemToString={x => pb.dateFormatToString(x) ?? 'N/A'}
                    renderSelectedItem={x => pb.dateFormatToString(x) ?? 'N/A'}
                    disabled={dateFormat.disabled}
                    invalid={!!dateFormat.error}
                    invalidText={dateFormat.error}
                />
            </Field>

            <Field
                variant="light"
                title={formatMessage({ defaultMessage: 'Number Format' })}
                disabled={numberFormat.disabled}
            >
                <Dropdown<NumberFormat>
                    size="md"
                    label=""
                    titleText=""
                    hideLabel
                    id={$('number-format')}
                    items={pb.numberFormatOptions}
                    selectedItem={numberFormat.value ?? undefined}
                    onChange={onNumberFormatChange}
                    itemToString={x => pb.numberFormatToString(x) ?? 'N/A'}
                    renderSelectedItem={x => pb.numberFormatToString(x) ?? 'N/A'}
                    disabled={numberFormat.disabled}
                    invalid={!!numberFormat.error}
                    invalidText={numberFormat.error}
                />
            </Field>

            <Field
                variant="light"
                title={formatMessage({ defaultMessage: 'Temperature' })}
                disabled={temperatureUnits.disabled}
            >
                <ButtonSwitch<TemperatureUnit>
                    id={$('temperature')}
                    size="md"
                    selectedOption={temperatureUnits.value}
                    options={temperatureOptions}
                    disabled={temperatureUnits.disabled}
                    onChange={temperatureUnits.onChange}
                    invalid={!!temperatureUnits.error}
                    invalidText={temperatureUnits.error}
                />
            </Field>

            <Field
                variant="light"
                title={formatMessage({ defaultMessage: 'Unit System' })}
                disabled={unitSystem.disabled}
            >
                <ButtonSwitch<UnitSystem>
                    id={$('unit-system')}
                    size="md"
                    selectedOption={unitSystem.value}
                    options={unitSystemOptions}
                    disabled={unitSystem.disabled}
                    onChange={unitSystem.onChange}
                    invalid={!!unitSystem.error}
                    invalidText={unitSystem.error}
                />
            </Field>
        </FieldSet>
    );
}

export interface PasswordFieldsProps {
    password1: iField<string>;
    password2: iField<string>;
}

/** The optional device password, entered twice. */
export function PasswordFields(props: PasswordFieldsProps & { $: IdOf }) {
    const { $, password1, password2 } = props;
    const { formatMessage } = useIntl();

    return (
        <FieldSet
            title={formatMessage({ defaultMessage: 'Password (optional)' })}
            description={formatMessage({
                defaultMessage: "If you forget this password, you'll need to reset the device to regain access.",
            })}
        >
            <Field
                variant="light"
                title={
                    <Tooltip
                        render={ref => (
                            <span ref={ref} className={css.withIcon}>
                                <span children={formatMessage({ defaultMessage: 'Password' })} />
                                <IconInfo size={14} />
                            </span>
                        )}
                        content={formatMessage({ defaultMessage: 'Password must be at least 6 characters long' })}
                        placement="top"
                    />
                }
                disabled={password1.disabled}
            >
                <PasswordInput
                    id={$('password-1')}
                    hideLabel
                    labelText={null}
                    tooltipPosition="left"
                    value={password1.value ?? ''}
                    onChange={e => password1.onChange?.(e.target.value)}
                    disabled={password1.disabled}
                    invalid={!!password1.error}
                    invalidText={password1.error}
                    placeholder="---"
                />
            </Field>

            <Field
                variant="light"
                title={formatMessage({ defaultMessage: 'Password Repeat' })}
                disabled={password2.disabled}
            >
                <PasswordInput
                    id={$('password-2')}
                    hideLabel
                    labelText={null}
                    tooltipPosition="left"
                    value={password2.value ?? ''}
                    onChange={e => password2.onChange?.(e.target.value)}
                    disabled={password2.disabled}
                    invalid={!!password2.error}
                    invalidText={password2.error}
                    placeholder="---"
                />
            </Field>
        </FieldSet>
    );
}
