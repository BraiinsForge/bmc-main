import { localTimeFormat } from '@/lib/time';
import type { Timestamp as PbTimestamp } from '@/proto';

const FAULT_MARKER = `invalid-datetime-value-${Math.random()}`;

export interface DatetimeProps {
    value: Maybe<Timestamp | PbTimestamp | bigint | Date>; // Posix timestamp (seconds)

    format?: Maybe<string>; // d3's format string
    seconds?: boolean; // determines if the default format string (used if none supplied) will include seconds
    tzname?: string; // Timezone name

    placeholder?: ReactNode;
    className?: string;
    style?: CSSProperties;
}
export function Datetime(props: DatetimeProps) {
    const { value, format, placeholder = '---', seconds, tzname, className, ...rest } = props;
    const $format = format || (seconds ? '%d.%m.%Y %H:%M:%S' : '%d.%m.%Y %H:%M');

    let res: ReactNode = placeholder;
    if (value != null) res = localTimeFormat(value, $format, tzname, FAULT_MARKER);
    if (res === FAULT_MARKER) res = placeholder;

    return <span {...rest} className={className} children={res} />;
}
