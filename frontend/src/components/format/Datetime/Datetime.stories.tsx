import { Datetime as Component, type DatetimeProps } from './Datetime';

const value = new Date(2017, 1, 15, 10, 20, 40);

export default {
    title: 'components/format/Datetime',
    component: Component,
};

export const Datetime = (args: DatetimeProps) => {
    return (
        <div style={{ fontSize: '2rem', color: '#fff' }}>
            <Component {...args} />
        </div>
    );
};
Datetime.args = {
    value: Math.floor(value.valueOf() / 1e3),
    format: '%d.%m.%Y %H:%M:%S',
    placeholder: '---',
    seconds: false,
    tzname: 'UTC',
} as DatetimeProps;
