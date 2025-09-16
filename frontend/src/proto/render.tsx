import { type CarbonIconType, TemperatureCelsius, TemperatureFahrenheit } from '@carbon/react/icons';

// Libs
import type { IntlShape } from 'react-intl';
import { assertUnreachable } from '@/lib/ts';
import { formatDuration } from 'date-fns';
import { isEqual } from 'es-toolkit';

// Proto
import { create } from '@bufbuild/protobuf';
import * as pb from './pb';

export function renderTimezone(tz: Maybe<pb.Timezone>): string {
    if (!tz) return 'N/A';
    return `UTC${tz.offset} (${tz.label})`;
}

export const wifiEncryptionTypeOptions: Array<Exclude<pb.EncryptionType, 0>> = [
    pb.EncryptionType.NONE,
    pb.EncryptionType.WEP,
    pb.EncryptionType.WEP_SHARED,
    pb.EncryptionType.WPA,
    pb.EncryptionType.WPA1_2,
    pb.EncryptionType.WPA2,
    pb.EncryptionType.WPA2_3,
];
export function wifiEncryptionTypeToString(intl: IntlShape, x?: Maybe<pb.EncryptionType>): null;
export function wifiEncryptionTypeToString(intl: IntlShape, x: Exclude<pb.EncryptionType, 0>): string;
export function wifiEncryptionTypeToString(intl: IntlShape, x?: null | pb.EncryptionType) {
    const { formatMessage } = intl;

    switch (x) {
        case null:
        case undefined:
        case pb.EncryptionType.UNSPECIFIED:
            return null;

        case pb.EncryptionType.NONE:
            return formatMessage({ defaultMessage: 'None' });

        case pb.EncryptionType.WEP:
            return formatMessage({ defaultMessage: 'WEP' });

        case pb.EncryptionType.WEP_SHARED:
            return formatMessage({ defaultMessage: 'WEP Shared' });

        case pb.EncryptionType.WPA:
            return formatMessage({ defaultMessage: 'WPA' });

        case pb.EncryptionType.WPA1_2:
            return formatMessage({ defaultMessage: 'WPA / WPA2' });

        case pb.EncryptionType.WPA2:
            return formatMessage({ defaultMessage: 'WPA2' });

        case pb.EncryptionType.WPA2_3:
            return formatMessage({ defaultMessage: 'WPA2 / WPA3' });

        case pb.EncryptionType.WPA3:
            return formatMessage({ defaultMessage: 'WPA3' });

        default:
            assertUnreachable(x, 'Wifi encryption type');
    }
}

export const weekdayOptionsAll: Array<Exclude<pb.Weekday, 0>> = [
    pb.Weekday.MONDAY,
    pb.Weekday.TUESDAY,
    pb.Weekday.WEDNESDAY,
    pb.Weekday.THURSDAY,
    pb.Weekday.FRIDAY,
    pb.Weekday.SATURDAY,
    pb.Weekday.SUNDAY,
];
export const weekdayOptionsWeek: Array<Exclude<pb.Weekday, 0>> = [
    pb.Weekday.MONDAY,
    pb.Weekday.TUESDAY,
    pb.Weekday.WEDNESDAY,
    pb.Weekday.THURSDAY,
    pb.Weekday.FRIDAY,
];
export const weekdayOptionsWeekend: Array<Exclude<pb.Weekday, 0>> = [pb.Weekday.SATURDAY, pb.Weekday.SUNDAY];
export function weekdayToString(intl: IntlShape, x?: Maybe<pb.Weekday>, long?: boolean): null;
export function weekdayToString(intl: IntlShape, x: Exclude<pb.Weekday, 0>, long?: boolean): string;
export function weekdayToString(intl: IntlShape, x?: null | pb.Weekday, long?: boolean) {
    const { formatMessage } = intl;

    switch (x) {
        case null:
        case undefined:
        case pb.Weekday.UNSPECIFIED:
            return null;

        case pb.Weekday.MONDAY:
            return long ? formatMessage({ defaultMessage: 'Monday' }) : formatMessage({ defaultMessage: 'Mon' });

        case pb.Weekday.TUESDAY:
            return long ? formatMessage({ defaultMessage: 'Tuesday' }) : formatMessage({ defaultMessage: 'Tue' });

        case pb.Weekday.WEDNESDAY:
            return long ? formatMessage({ defaultMessage: 'Wednesday' }) : formatMessage({ defaultMessage: 'Wed' });

        case pb.Weekday.THURSDAY:
            return long ? formatMessage({ defaultMessage: 'Thursday' }) : formatMessage({ defaultMessage: 'Thu' });

        case pb.Weekday.FRIDAY:
            return long ? formatMessage({ defaultMessage: 'Friday' }) : formatMessage({ defaultMessage: 'Fri' });

        case pb.Weekday.SATURDAY:
            return long ? formatMessage({ defaultMessage: 'Saturday' }) : formatMessage({ defaultMessage: 'Sat' });

        case pb.Weekday.SUNDAY:
            return long ? formatMessage({ defaultMessage: 'Sunday' }) : formatMessage({ defaultMessage: 'Sun' });

        default:
            assertUnreachable(x, 'Scene transition effect');
    }
}
export function weekdayListToString(intl: IntlShape, x: Maybe<pb.Weekday[]>): null | string {
    if (!x) return null;

    const { formatMessage } = intl;
    const unique: pb.Weekday[] = Array.from(new Set(x.filter(Boolean))).sort();

    if (isEqual(unique, weekdayOptionsAll)) return formatMessage({ defaultMessage: 'All days' });
    if (isEqual(unique, weekdayOptionsWeek)) return formatMessage({ defaultMessage: 'Weekdays' });
    if (isEqual(unique, weekdayOptionsWeekend)) return formatMessage({ defaultMessage: 'Weekends' });
    return unique
        .toSorted()
        .map(x => weekdayToString(intl, x))
        .join(', ');
}

export function alarmSnoozeOptionsToString(intl: IntlShape, snoozeOptions: Maybe<pb.SnoozeOptionsWrapper>) {
    const { formatMessage } = intl;
    return snoozeOptions?.kind?.case === 'snooze'
        ? formatMessage(
              { defaultMessage: 'On, {duration}, {limit}' },
              {
                  duration: alarmSnoozeDurationToString(intl, snoozeOptions.kind.value.duration),
                  limit: alarmSnoozeLimitToString(intl, snoozeOptions.kind.value.limit),
              },
          )
        : formatMessage({ defaultMessage: 'Off' });
}

export const alarmSnoozeLimitOptions: Array<Exclude<pb.SnoozeLimit, 0>> = [
    pb.SnoozeLimit.SNOOZE_LIMIT_FOREVER,
    pb.SnoozeLimit.SNOOZE_LIMIT_3,
    pb.SnoozeLimit.SNOOZE_LIMIT_5,
];
export function alarmSnoozeLimitToString(intl: IntlShape, x?: Maybe<pb.SnoozeLimit>): null;
export function alarmSnoozeLimitToString(intl: IntlShape, x: Exclude<pb.SnoozeLimit, 0>): string;
export function alarmSnoozeLimitToString(intl: IntlShape, x?: null | pb.SnoozeLimit) {
    const { formatMessage } = intl;

    switch (x) {
        case null:
        case undefined:
        case pb.SnoozeLimit.SNOOZE_LIMIT_UNSPECIFIED:
            return null;

        case pb.SnoozeLimit.SNOOZE_LIMIT_FOREVER:
            return formatMessage({ defaultMessage: 'Forever' });

        case pb.SnoozeLimit.SNOOZE_LIMIT_3:
            return formatMessage({ defaultMessage: '3 snoozes' });

        case pb.SnoozeLimit.SNOOZE_LIMIT_5:
            return formatMessage({ defaultMessage: '5 snoozes' });

        default:
            assertUnreachable(x, 'alarm snooze limit');
    }
}

export const alarmSnoozeDurationOptions: Array<Exclude<pb.SnoozeDuration, 0>> = [
    pb.SnoozeDuration.SNOOZE_DURATION_5_MINUTES,
    pb.SnoozeDuration.SNOOZE_DURATION_10_MINUTES,
    pb.SnoozeDuration.SNOOZE_DURATION_15_MINUTES,
    pb.SnoozeDuration.SNOOZE_DURATION_30_MINUTES,
];
export function alarmSnoozeDurationToString(intl: IntlShape, x?: Maybe<pb.SnoozeDuration>): null;
export function alarmSnoozeDurationToString(intl: IntlShape, x: Exclude<pb.SnoozeDuration, 0>): string;
export function alarmSnoozeDurationToString(intl: IntlShape, x?: null | pb.SnoozeDuration) {
    const { formatMessage } = intl;

    switch (x) {
        case null:
        case undefined:
        case pb.SnoozeDuration.SNOOZE_DURATION_UNSPECIFIED:
            return null;

        case pb.SnoozeDuration.SNOOZE_DURATION_5_MINUTES:
            return formatMessage({ defaultMessage: '5 minutes' });

        case pb.SnoozeDuration.SNOOZE_DURATION_10_MINUTES:
            return formatMessage({ defaultMessage: '10 minutes' });

        case pb.SnoozeDuration.SNOOZE_DURATION_15_MINUTES:
            return formatMessage({ defaultMessage: '15 minutes' });

        case pb.SnoozeDuration.SNOOZE_DURATION_30_MINUTES:
            return formatMessage({ defaultMessage: '30 minutes' });

        default:
            assertUnreachable(x, 'alarm snooze duration');
    }
}

export const sceneCyclingEffectOptions: Array<Exclude<pb.SceneCyclingTransition, 0>> = [
    pb.SceneCyclingTransition.FADE,
    pb.SceneCyclingTransition.SLIDE,
];
export function sceneCyclingEffectToString(intl: IntlShape, x?: Maybe<pb.SceneCyclingTransition>): null;
export function sceneCyclingEffectToString(intl: IntlShape, x: Exclude<pb.SceneCyclingTransition, 0>): string;
export function sceneCyclingEffectToString(intl: IntlShape, x?: null | pb.SceneCyclingTransition) {
    const { formatMessage } = intl;

    switch (x) {
        case null:
        case undefined:
        case pb.SceneCyclingTransition.UNSPECIFIED:
            return null;

        case pb.SceneCyclingTransition.FADE:
            return formatMessage({ defaultMessage: 'Fade' });

        case pb.SceneCyclingTransition.SLIDE:
            return formatMessage({ defaultMessage: 'Slide' });

        default:
            assertUnreachable(x, 'Scene transition effect');
    }
}

export const tickerTimeFrameOptions: Array<Exclude<pb.TickerBtcWidget_TimeFrame, 0>> = [
    pb.TickerBtcWidget_TimeFrame.DAY_1,
    pb.TickerBtcWidget_TimeFrame.WEEK_1,
    pb.TickerBtcWidget_TimeFrame.WEEK_2,
    pb.TickerBtcWidget_TimeFrame.MONTH_1,
    pb.TickerBtcWidget_TimeFrame.MONTH_3,
    pb.TickerBtcWidget_TimeFrame.MONTH_6,
    pb.TickerBtcWidget_TimeFrame.YEAR_1,
    pb.TickerBtcWidget_TimeFrame.YEAR_2,
    pb.TickerBtcWidget_TimeFrame.YEAR_5,
    pb.TickerBtcWidget_TimeFrame.ALL,
];
export function tickerTimeFrameToString(intl: IntlShape, x?: Maybe<pb.TickerBtcWidget_TimeFrame>): null;
export function tickerTimeFrameToString(intl: IntlShape, x: Exclude<pb.TickerBtcWidget_TimeFrame, 0>): string;
export function tickerTimeFrameToString(intl: IntlShape, x?: null | pb.TickerBtcWidget_TimeFrame) {
    const { formatMessage } = intl;

    switch (x) {
        case null:
        case undefined:
        case pb.TickerBtcWidget_TimeFrame.UNSPECIFIED:
            return null;

        case pb.TickerBtcWidget_TimeFrame.DAY_1:
            return formatMessage({ defaultMessage: '1 Day' });

        case pb.TickerBtcWidget_TimeFrame.WEEK_1:
            return formatMessage({ defaultMessage: '1 Week' });

        case pb.TickerBtcWidget_TimeFrame.WEEK_2:
            return formatMessage({ defaultMessage: '2 Weeks' });

        case pb.TickerBtcWidget_TimeFrame.MONTH_1:
            return formatMessage({ defaultMessage: '1 Month' });

        case pb.TickerBtcWidget_TimeFrame.MONTH_3:
            return formatMessage({ defaultMessage: '3 Months' });

        case pb.TickerBtcWidget_TimeFrame.MONTH_6:
            return formatMessage({ defaultMessage: '6 Months' });

        case pb.TickerBtcWidget_TimeFrame.YEAR_1:
            return formatMessage({ defaultMessage: '1 Year' });

        case pb.TickerBtcWidget_TimeFrame.YEAR_2:
            return formatMessage({ defaultMessage: '2 Years' });

        case pb.TickerBtcWidget_TimeFrame.YEAR_5:
            return formatMessage({ defaultMessage: '5 Years' });

        case pb.TickerBtcWidget_TimeFrame.ALL:
            return formatMessage({ defaultMessage: 'All' });

        default:
            assertUnreachable(x, 'Scene transition effect');
    }
}

export const sceneCycleDurationOptions: number[] = [10, 20, 30, 40, 50, 60, 90, 120];
export function sceneCycleDurationToString(value: Maybe<number>): string {
    if (value == null) return 'N/A';
    const minutes = Math.floor(value / 60);
    const seconds = Math.floor(value - minutes * 60);
    return formatDuration({ minutes, seconds }, { format: ['minutes', 'seconds'] });
}

/**
 * The selection dropdown is a Downshift instance,
 * which requires all options to be of the same type.
 *
 * This is a placeholder object that represents the "Other…" option.
 * It needs to be caught and handled in special way in…
 *  - renderToString
 *  - renderToElement
 *  - onChange
 */
export const WIFI_AP_OTHER_PLACEHOLDER: Readonly<pb.WifiNetwork> = Object.freeze(
    create(pb.WifiNetworkSchema, { ssid: 'WIFI_AP_OTHER_PLACEHOLDER' }),
);
export function wifiNetworkToString(net: Maybe<pb.WifiNetwork>, other: string): null | string {
    if (net == null) return null;
    return net.ssid === WIFI_AP_OTHER_PLACEHOLDER.ssid ? other : net.ssid;
}

export const dateFormatOptions: Array<Exclude<pb.DateFormat, 0>> = [
    pb.DateFormat.DD_MM_YYYY_DOT,
    pb.DateFormat.DD_MM_YYYY_SLASH,
    pb.DateFormat.D_M_YYYY_SLASH,
    pb.DateFormat.M_D_YYYY_SLASH,
    pb.DateFormat.DD_MM_YYYY_DASH,
    pb.DateFormat.YYYY_M_D_SLASH,
    pb.DateFormat.YYYY_MM_DD_DOT,
    pb.DateFormat.YYYY_MM_DD_DASH,
];
export function dateFormatToString(x: Maybe<pb.DateFormat>): null;
export function dateFormatToString(x: Exclude<pb.DateFormat, 0>): string;
export function dateFormatToString(x: Maybe<pb.DateFormat>) {
    switch (x) {
        case null:
        case undefined:
        case pb.DateFormat.UNSPECIFIED:
            return null;

        case pb.DateFormat.DD_MM_YYYY_DOT:
            return 'DD.MM.YYYY' as string;

        case pb.DateFormat.DD_MM_YYYY_SLASH:
            return 'DD/MM/YYYY';

        case pb.DateFormat.D_M_YYYY_SLASH:
            return 'D/M/YYYY';

        case pb.DateFormat.M_D_YYYY_SLASH:
            return 'M/D/YYYY';

        case pb.DateFormat.DD_MM_YYYY_DASH:
            return 'DD-MM-YYYY';

        case pb.DateFormat.YYYY_M_D_SLASH:
            return 'YYYY/M/D';

        case pb.DateFormat.YYYY_MM_DD_DOT:
            return 'YYYY.MM.DD';

        case pb.DateFormat.YYYY_MM_DD_DASH:
            return 'YYYY-MM-DD';

        default:
            assertUnreachable(x, 'date format');
    }
}

export const temperatureUnitOptions: Map<Exclude<pb.TemperatureUnit, 0>, CarbonIconType> = new Map([
    [pb.TemperatureUnit.CELSIUS, TemperatureCelsius],
    [pb.TemperatureUnit.FAHRENHEIT, TemperatureFahrenheit],
]);
export function temperatureUnitToString(intl: IntlShape, x?: Maybe<pb.TemperatureUnit>): null;
export function temperatureUnitToString(intl: IntlShape, x: Exclude<pb.TemperatureUnit, 0>): string;
export function temperatureUnitToString(intl: IntlShape, x?: null | pb.TemperatureUnit) {
    switch (x) {
        case null:
        case undefined:
        case pb.TemperatureUnit.UNSPECIFIED:
            return null;

        case pb.TemperatureUnit.CELSIUS:
            return intl.formatMessage({ defaultMessage: 'Celsius' });

        case pb.TemperatureUnit.FAHRENHEIT:
            return intl.formatMessage({ defaultMessage: 'Fahrenheit' });

        default:
            assertUnreachable(x, 'temperature unit');
    }
}

export const numberFormatOptions: Array<Exclude<pb.NumberFormat, 0>> = [
    pb.NumberFormat.SPACE_GROUP_COMMA_DECIMAL,
    pb.NumberFormat.COMMA_GROUP_DOT_DECIMAL,
    pb.NumberFormat.DOT_GROUP_COMMA_DECIMAL,
    pb.NumberFormat.SPACE_GROUP_DOT_DECIMAL,
];
export function numberFormatToString(x: Maybe<pb.NumberFormat>): null | string {
    if (!x) return null;
    switch (x) {
        case pb.NumberFormat.SPACE_GROUP_COMMA_DECIMAL:
            return '1 234 567,89';

        case pb.NumberFormat.COMMA_GROUP_DOT_DECIMAL:
            return '1,234,567.89';

        case pb.NumberFormat.DOT_GROUP_COMMA_DECIMAL:
            return '1.234.567,89';

        case pb.NumberFormat.SPACE_GROUP_DOT_DECIMAL:
            return '1 234 567.89';

        default:
            assertUnreachable(x, 'number format');
    }
}

export function clockStyleToString(intl: IntlShape, style: pb.ClockWidget_ClockStyle): null | string {
    if (!style) return null;
    switch (style) {
        case pb.ClockWidget_ClockStyle.ANALOG_ROUND:
            return intl.formatMessage({ defaultMessage: 'Analog (round)' });

        case pb.ClockWidget_ClockStyle.ANALOG_RECT:
            return intl.formatMessage({ defaultMessage: 'Analog (rectangular)' });

        case pb.ClockWidget_ClockStyle.DIGITAL:
            return intl.formatMessage({ defaultMessage: 'Digital' });

        default:
            assertUnreachable(style, 'clock style');
    }
}

export const fontStyleOptions: Array<Exclude<pb.FontStyle, 0>> = [
    pb.FontStyle.LIGHT,
    pb.FontStyle.MEDIUM,
    pb.FontStyle.BOLD,
] as const;
export function fontStyleToString(intl: IntlShape, x?: Maybe<pb.FontStyle>): null;
export function fontStyleToString(intl: IntlShape, x: Exclude<pb.FontStyle, 0>): string;
export function fontStyleToString(intl: IntlShape, x?: null | pb.FontStyle) {
    switch (x) {
        case null:
        case undefined:
        case pb.FontStyle.UNSPECIFIED:
            return null;

        case pb.FontStyle.LIGHT:
            return intl.formatMessage({ defaultMessage: 'Light' });

        case pb.FontStyle.MEDIUM:
            return intl.formatMessage({ defaultMessage: 'Medium' });

        case pb.FontStyle.BOLD:
            return intl.formatMessage({ defaultMessage: 'Bold' });

        default:
            assertUnreachable(x, 'clock style');
    }
}

export const accountTypeOptions: Array<Exclude<pb.AccountType, 0>> = [pb.AccountType.BRAIINSPOOL];
export function accountTypeToString(intl: IntlShape, x?: Maybe<pb.AccountType>): null;
export function accountTypeToString(intl: IntlShape, x: Exclude<pb.AccountType, 0>): string;
export function accountTypeToString(intl: IntlShape, x?: null | pb.AccountType) {
    switch (x) {
        case null:
        case undefined:
        case pb.AccountType.UNSPECIFIED:
            return null;

        case pb.AccountType.BRAIINSPOOL:
            return intl.formatMessage({ defaultMessage: 'Braiins Pool' });

        default:
            assertUnreachable(x, 'clock style');
    }
}

type BraiinsPoolStyle = pb.BraiinsPoolWidget_BraiinsPoolStyle;
export const braiinsPoolStyleOptions: Array<Exclude<BraiinsPoolStyle, 0>> = [
    pb.BraiinsPoolWidget_BraiinsPoolStyle.OVERVIEW,
    pb.BraiinsPoolWidget_BraiinsPoolStyle.BIGCHART,
];
export function braiinsPoolStyleToString(intl: IntlShape, x?: Maybe<BraiinsPoolStyle>): null;
export function braiinsPoolStyleToString(intl: IntlShape, x: Exclude<BraiinsPoolStyle, 0>): string;
export function braiinsPoolStyleToString(intl: IntlShape, x?: null | BraiinsPoolStyle) {
    switch (x) {
        case null:
        case undefined:
        case pb.BraiinsPoolWidget_BraiinsPoolStyle.UNSPECIFIED:
            return null;

        case pb.BraiinsPoolWidget_BraiinsPoolStyle.OVERVIEW:
            return intl.formatMessage({ defaultMessage: 'Overview' });

        case pb.BraiinsPoolWidget_BraiinsPoolStyle.BIGCHART:
            return intl.formatMessage({ defaultMessage: 'Big Chart' });

        default:
            assertUnreachable(x, 'view style');
    }
}

type BraiinsPoolTimeFrame = pb.BraiinsPoolWidget_TimeFrame;
export const braiinsPoolTimeFrameOptions: Array<Exclude<BraiinsPoolTimeFrame, 0>> = [
    pb.BraiinsPoolWidget_TimeFrame.HOUR_4,
    pb.BraiinsPoolWidget_TimeFrame.HOUR_12,
    pb.BraiinsPoolWidget_TimeFrame.HOUR_24,
    pb.BraiinsPoolWidget_TimeFrame.DAY_7,
];
export function braiinsPoolTimeFrameToString(intl: IntlShape, x?: Maybe<BraiinsPoolTimeFrame>): null;
export function braiinsPoolTimeFrameToString(intl: IntlShape, x: Exclude<BraiinsPoolTimeFrame, 0>): string;
export function braiinsPoolTimeFrameToString(intl: IntlShape, x?: null | BraiinsPoolTimeFrame) {
    const { formatMessage } = intl;

    switch (x) {
        case null:
        case undefined:
        case pb.BraiinsPoolWidget_TimeFrame.UNSPECIFIED:
            return null;

        case pb.BraiinsPoolWidget_TimeFrame.HOUR_4:
            return formatMessage({ defaultMessage: '4 Hours' });

        case pb.BraiinsPoolWidget_TimeFrame.HOUR_12:
            return formatMessage({ defaultMessage: '12 Hours' });

        case pb.BraiinsPoolWidget_TimeFrame.HOUR_24:
            return formatMessage({ defaultMessage: '24 Hours' });

        case pb.BraiinsPoolWidget_TimeFrame.DAY_7:
            return formatMessage({ defaultMessage: '7 Days' });

        default:
            assertUnreachable(x, 'Scene transition effect');
    }
}

export function sceneTitle(intl: IntlShape, kind: Maybe<ProtoOneofCase<pb.WidgetKind['value']>>): null | string {
    switch (kind) {
        case null:
        case undefined:
            return null;

        case 'clock':
            return intl.formatMessage({ defaultMessage: 'Clock' });

        case 'tickerBtc':
            return intl.formatMessage({ defaultMessage: 'Bitcoin Ticker' });

        case 'blockHeight':
            return intl.formatMessage({ defaultMessage: 'Block Height' });

        case 'braiinsPool':
            return intl.formatMessage({ defaultMessage: 'Braiins Pool' });

        default:
            assertUnreachable(kind);
    }
}

export function widgetDescription(intl: IntlShape, data: Maybe<pb.WidgetKind>) {
    const { formatMessage } = intl;
    const val = data?.value;
    switch (val?.case) {
        case undefined:
            return null;

        case 'clock':
            return intl.formatMessage(
                { defaultMessage: 'Style: {style}, font: {font}' },
                {
                    style: clockStyleToString(intl, val.value.clockStyle) || 'N/A',
                    font: fontStyleToString(intl, val.value.numbersFontStyle) || 'N/A',
                },
            );

        case 'tickerBtc':
            return intl.formatMessage(
                { defaultMessage: 'Time frame: {timeframe}' },
                {
                    timeframe: tickerTimeFrameToString(intl, val.value.timeFrame) || 'N/A',
                },
            );

        case 'blockHeight':
            return intl.formatMessage(
                { defaultMessage: 'Time & date: {dateTime}, font: {font}' },
                {
                    dateTime: val.value.showTimestamp
                        ? formatMessage({ defaultMessage: 'Yes' })
                        : formatMessage({ defaultMessage: 'No' }),
                    font: fontStyleToString(intl, val.value.numbersFontStyle) || 'N/A',
                },
            );

        case 'braiinsPool': {
            const d = val.value satisfies pb.BraiinsPoolWidget;
            return intl.formatMessage(
                { defaultMessage: 'Time frame: {timeframe}' },
                {
                    timeframe: braiinsPoolTimeFrameToString(intl, d.timeFrame) || 'N/A',
                },
            );
        }

        default:
            assertUnreachable(val, 'fullscreen widget kind');
    }
}
export function sceneDescription(intl: IntlShape, data: Maybe<MaybeArray<pb.Widget>>): null | string {
    if (data == null) return null;

    // Combined scene
    if (Array.isArray(data)) {
        return data
            .map(x => sceneTitle(intl, x.kind?.value.case))
            .filter(Boolean)
            .join(', ');
    }

    // Fullscreen widget
    else if (data) return widgetDescription(intl, data.kind);
    // Fail
    else assertUnreachable(data);
}
