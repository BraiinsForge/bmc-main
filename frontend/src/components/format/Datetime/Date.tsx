import { Datetime, type DatetimeProps } from './Datetime';

export type DateProps = Omit<DatetimeProps, 'format' | 'tzname' | 'seconds'>;
export function Date(props: DateProps) {
    return <Datetime {...props} tzname="UTC" format="%d.%m.%Y" />;
}
