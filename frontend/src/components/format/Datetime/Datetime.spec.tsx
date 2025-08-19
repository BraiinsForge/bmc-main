import { beforeEach, describe, test, expect } from '@rstest/core';
import { cleanup, render } from '@testing-library/react/pure';

import { Datetime, type DatetimeProps } from './Datetime';

beforeEach(cleanup);

function renderDatetime(props: DatetimeProps) {
    const utils = render(<Datetime {...props} />);

    return {
        ...utils,
        time: () => utils.baseElement.querySelector('span'),
    };
}

const props = {
    value: 1571665269,
    format: '%d.%m.%Y %H:%M:%S',
    placeholder: 'placeholder',
    seconds: false,
    tzname: 'UTC',
};

describe('<Datetime />', () => {
    test('checks if a timestamp is changed into a correct format', () => {
        const { time } = renderDatetime({ ...props });

        expect(time()?.textContent).toEqual('21.10.2019 13:41:09');
    });
});
