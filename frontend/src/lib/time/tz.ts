export interface TimezoneOption {
    value: string;
    label: string;
}

export function formatTimezoneLabel(
    locale: Intl.UnicodeBCP47LocaleIdentifier,
    timeZoneName: string,
    // Usually probably not provided externally,
    // but much more efficient when formatting
    // A LOT of timezones
    __now__: Date = new Date(),
) {
    const formatter = new Intl.DateTimeFormat(locale, {
        timeZone: timeZoneName,
        timeZoneName: 'short',
        hour: '2-digit',
        minute: '2-digit',
    });
    const dateAndOffset = formatter.format(__now__);
    return `${timeZoneName} (${dateAndOffset})`;
}

export function getTimeZonesWithLabels<R = TimezoneOption>(
    locale: Intl.UnicodeBCP47LocaleIdentifier = 'en-UK',
    mapFunction?: (item: TimezoneOption) => R,
): R[] {
    const timeZones = Intl.supportedValuesOf('timeZone');
    const now = new Date();

    return timeZones.map<R>(tzName => {
        const res: TimezoneOption = {
            value: tzName,
            label: formatTimezoneLabel(locale, tzName, now),
        };
        return (mapFunction ? mapFunction(res) : res) as R;
    });
}
