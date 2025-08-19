import { localTimeFormat } from '@/lib/time';

export type DatetimeProps = {
    value: Maybe<Timestamp | bigint | Date>; // Posix timestamp (seconds)

    format?: Maybe<string>; // d3's format string
    seconds?: boolean; // determines if the default format string (used if none supplied) will include seconds
    tzname?: string; // Timezone name

    placeholder?: ReactNode;
    className?: string;
    style?: CSSProperties;
};

export function Datetime(props: DatetimeProps) {
    const { value, format, placeholder = '---', seconds, tzname, className, ...rest } = props;

    const res = (val: ReactNode) => <span role="timer" {...rest} className={className} children={val} />;

    if (!value || (!Number.isFinite(value) && !(value instanceof Date))) return res(placeholder);
    const fmt = format || (seconds ? '%d.%m.%Y %H:%M:%S' : '%d.%m.%Y %H:%M');
    return res(localTimeFormat(value, fmt, tzname));
}
