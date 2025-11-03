import { useState, useCallback } from 'react';
import { startCase } from 'es-toolkit';
import { action } from 'storybook/actions';
import type { Meta } from '@storybook/react';

import * as pb from '@/proto';
import type * as t from './types';
import { FormWidgetRemoteWidget as Component, type FormWidgetRemoteWidgetProps } from './FormWidgetRemoteWidget';

export default {
    title: 'display/components/FormWidgetRemoteWidget',
    component: Component,
} satisfies Meta<FormWidgetRemoteWidgetProps>;

const isInvalid: boolean = false;
const err = (text: string): null | string => (isInvalid ? text : null);

function Demo() {
    type StateProps = Exclude<
        keyof FormWidgetRemoteWidgetProps,
        'isOpen' | 'isEdit' | 'onClose' | 'onSubmit' | 'error'
    >;
    const handleChange = useCallback(<Key extends StateProps>(key: Key) => {
        return (value: any) => {
            setArgs(prev => ({ ...prev, [key]: { ...prev[key], value } }));
        };
    }, []);
    const [args, setArgs] = useState<FormWidgetRemoteWidgetProps>({
        isOpen: true,
        isEdit: false,
        onClose: action('onClose'),
        error: err('Global error'),

        widgetSize: {
            value: pb.WidgetSize.MEDIUM,
            disabled: false,
            error: err('The bliss is an ultimate believer!'),
            onChange: handleChange('widgetSize'),
            options: [pb.WidgetSize.SMALL, pb.WidgetSize.MEDIUM, pb.WidgetSize.LARGE],
        },
        url: { value: 'http://127.0.0.1:8080/debug' },
        name: { value: '' },
        params: { value: {} },
    });

    return <Component {...args} style={{ padding: 18, backgroundColor: 'var(--cds-layer-01)' }} />;
}
export function Playground() {
    return <Demo />;
}

interface ExampleProps {
    url: string;
    params: Record<string, t.Param>;
}
function Example({ url, params }: ExampleProps) {
    const [paramsValue, setParamsValue] = useState<pb.JsonObject>({});
    return (
        <Component
            isOpen
            isEdit={false}
            onClose={action('onClose')}
            error={err('Global error')}
            widgetSize={null}
            url={{ value: url }}
            name={{ value: startCase(new URL(url).pathname.slice(1).replace(/-/g, ' ')) }}
            params={{ value: paramsValue, onChange: setParamsValue }}
            // Local params schema
            paramsSchema={params}
        />
    );
}

const dataDebug: ExampleProps = {
    url: 'http://127.0.0.1:8080/debug',
    params: {
        stringParam: {
            type: 'string',
            name: 'String',
            description: 'Man happens when you need acceptance so harmoniously.',
            default: 'default',
        },
        stringEnum: {
            type: 'string',
            name: 'String Enum',
            description: 'Confucius says: the everything of relativity leads to music.',
            default: 'option1',
            enum: ['option1', 'option2', 'option3'],
        },
        numberParam: {
            type: 'number',
            name: 'Number',
            description: 'Followers, saints, and private monkeys will always protect them.',
            default: 42.5,
        },
        integerParam: {
            type: 'integer',
            name: 'Integer',
            description: 'When one hurts politics and density, one is able to fear extend.',
            default: 42,
        },
        numberEnum: {
            type: 'number',
            name: 'Number Enum',
            description: 'Spiritual blisses shapes most stigmas.',
            default: 15,
            enum: [15, 42, 687],
        },
        booleanParam: {
            type: 'boolean',
            name: 'Boolean',
            description: 'Who can avoid the vision and advice?',
            default: true,
        },
        emailParam: {
            type: 'string',
            format: 'email',
            name: 'Email',
            description: 'Bliss doesn’t gently know any lord — but the master is what fails.',
            default: 'test@example.com',
        },
        stringArrayParam: {
            type: 'array',
            name: 'String Array',
            description: 'The meditation is a unprepared karma.',
            default: ['item1', 'item2', 'item3'],
            items: { type: 'string' },
        },
        numberArrayParam: {
            type: 'array',
            name: 'Number Array',
            description: 'Believers, scholars, and prime moons will always protect them.',
            default: [10, 20, 30],
            items: { type: 'number', minimum: 0, maximum: 100 },
        },
        enumArrayParam: {
            type: 'array',
            name: 'Enum Array',
            description: 'When one yearns pain and art, one is able to grasp core.',
            minItems: 2,
            maxItems: 3,
            items: { type: 'string', enum: ['option1', 'option2', 'option3'] },
        },
    },
};
const dataExchangeRate: ExampleProps = {
    url: 'http://127.0.0.1:8080/exchange-rate',
    params: {
        currencyOutput: {
            type: 'string',
            name: 'Output Currency',
            default: 'USD',
            description: 'Currency to convert to',
            enum: [
                'USD',
                'AED',
                'AFN',
                'ALL',
                'AMD',
                'ANG',
                'AOA',
                'ARS',
                'AUD',
                'AWG',
                'AZN',
                'BAM',
                'BBD',
                'BDT',
                'BGN',
                'BHD',
                'BIF',
                'BMD',
                'BND',
                'BOB',
                'BRL',
                'BSD',
                'BTN',
                'BWP',
                'BYN',
                'BZD',
                'CAD',
                'CDF',
                'CHF',
                'CLP',
                'CNY',
                'COP',
                'CRC',
                'CUP',
                'CVE',
                'CZK',
                'DJF',
                'DKK',
                'DOP',
                'DZD',
                'EGP',
                'ERN',
                'ETB',
                'EUR',
                'FJD',
                'FKP',
                'FOK',
                'GBP',
                'GEL',
                'GGP',
                'GHS',
                'GIP',
                'GMD',
                'GNF',
                'GTQ',
                'GYD',
                'HKD',
                'HNL',
                'HRK',
                'HTG',
                'HUF',
                'IDR',
                'ILS',
                'IMP',
                'INR',
                'IQD',
                'IRR',
                'ISK',
                'JEP',
                'JMD',
                'JOD',
                'JPY',
                'KES',
                'KGS',
                'KHR',
                'KID',
                'KMF',
                'KRW',
                'KWD',
                'KYD',
                'KZT',
                'LAK',
                'LBP',
                'LKR',
                'LRD',
                'LSL',
                'LYD',
                'MAD',
                'MDL',
                'MGA',
                'MKD',
                'MMK',
                'MNT',
                'MOP',
                'MRU',
                'MUR',
                'MVR',
                'MWK',
                'MXN',
                'MYR',
                'MZN',
                'NAD',
                'NGN',
                'NIO',
                'NOK',
                'NPR',
                'NZD',
                'OMR',
                'PAB',
                'PEN',
                'PGK',
                'PHP',
                'PKR',
                'PLN',
                'PYG',
                'QAR',
                'RON',
                'RSD',
                'RUB',
                'RWF',
                'SAR',
                'SBD',
                'SCR',
                'SDG',
                'SEK',
                'SGD',
                'SHP',
                'SLE',
                'SLL',
                'SOS',
                'SRD',
                'SSP',
                'STN',
                'SYP',
                'SZL',
                'THB',
                'TJS',
                'TMT',
                'TND',
                'TOP',
                'TRY',
                'TTD',
                'TVD',
                'TWD',
                'TZS',
                'UAH',
                'UGX',
                'UYU',
                'UZS',
                'VES',
                'VND',
                'VUV',
                'WST',
                'XAF',
                'XCD',
                'XCG',
                'XDR',
                'XOF',
                'XPF',
                'YER',
                'ZAR',
                'ZMW',
                'ZWL',
            ],
        },
        currencyInput: {
            type: 'string',
            name: 'Input Currency',
            default: 'EUR',
            description: 'Currency to convert from',
            enum: [
                'USD',
                'AED',
                'AFN',
                'ALL',
                'AMD',
                'ANG',
                'AOA',
                'ARS',
                'AUD',
                'AWG',
                'AZN',
                'BAM',
                'BBD',
                'BDT',
                'BGN',
                'BHD',
                'BIF',
                'BMD',
                'BND',
                'BOB',
                'BRL',
                'BSD',
                'BTN',
                'BWP',
                'BYN',
                'BZD',
                'CAD',
                'CDF',
                'CHF',
                'CLP',
                'CNY',
                'COP',
                'CRC',
                'CUP',
                'CVE',
                'CZK',
                'DJF',
                'DKK',
                'DOP',
                'DZD',
                'EGP',
                'ERN',
                'ETB',
                'EUR',
                'FJD',
                'FKP',
                'FOK',
                'GBP',
                'GEL',
                'GGP',
                'GHS',
                'GIP',
                'GMD',
                'GNF',
                'GTQ',
                'GYD',
                'HKD',
                'HNL',
                'HRK',
                'HTG',
                'HUF',
                'IDR',
                'ILS',
                'IMP',
                'INR',
                'IQD',
                'IRR',
                'ISK',
                'JEP',
                'JMD',
                'JOD',
                'JPY',
                'KES',
                'KGS',
                'KHR',
                'KID',
                'KMF',
                'KRW',
                'KWD',
                'KYD',
                'KZT',
                'LAK',
                'LBP',
                'LKR',
                'LRD',
                'LSL',
                'LYD',
                'MAD',
                'MDL',
                'MGA',
                'MKD',
                'MMK',
                'MNT',
                'MOP',
                'MRU',
                'MUR',
                'MVR',
                'MWK',
                'MXN',
                'MYR',
                'MZN',
                'NAD',
                'NGN',
                'NIO',
                'NOK',
                'NPR',
                'NZD',
                'OMR',
                'PAB',
                'PEN',
                'PGK',
                'PHP',
                'PKR',
                'PLN',
                'PYG',
                'QAR',
                'RON',
                'RSD',
                'RUB',
                'RWF',
                'SAR',
                'SBD',
                'SCR',
                'SDG',
                'SEK',
                'SGD',
                'SHP',
                'SLE',
                'SLL',
                'SOS',
                'SRD',
                'SSP',
                'STN',
                'SYP',
                'SZL',
                'THB',
                'TJS',
                'TMT',
                'TND',
                'TOP',
                'TRY',
                'TTD',
                'TVD',
                'TWD',
                'TZS',
                'UAH',
                'UGX',
                'UYU',
                'UZS',
                'VES',
                'VND',
                'VUV',
                'WST',
                'XAF',
                'XCD',
                'XCG',
                'XDR',
                'XOF',
                'XPF',
                'YER',
                'ZAR',
                'ZMW',
                'ZWL',
            ],
        },
    },
};
const dataNameday: ExampleProps = {
    url: 'http://127.0.0.1:8080/nameday',
    params: {
        country: {
            type: 'string',
            default: 'cz',
            description: 'Country code for nameday lookup',
            enum: [
                'at',
                'bg',
                'cz',
                'de',
                'dk',
                'ee',
                'es',
                'fi',
                'fr',
                'gr',
                'hr',
                'hu',
                'it',
                'lt',
                'lv',
                'pl',
                'ru',
                'se',
                'sk',
                'us',
            ],
            name: 'Country',
        },
    },
};
const dataTickerList: ExampleProps = {
    url: 'http://127.0.0.1:8080/ticker-list',
    params: {
        period: {
            type: 'string',
            default: '24h',
            description: 'Time period for price change calculation',
            enum: ['1h', '24h', '7d', '30d'],
            name: 'Time Period',
        },
        symbols: {
            type: 'string',
            name: 'Symbols',
            default: '["AAPL","^GSPC","BTC-USD","ETH-USD","NVDA","TSLA"]',
            description: 'List of financial symbols to display',
        },
    },
};
const dataTickerCandle: ExampleProps = {
    url: 'http://127.0.0.1:8080/ticker-candle',
    params: {
        period: {
            type: 'string',
            default: '24h',
            description: 'Time period for candlestick chart',
            enum: ['1h', '24h', '7d', '30d'],
            name: 'Time Period',
        },
        pair: {
            type: 'string',
            default: 'BTC-USD',
            description: 'Financial symbol, stock ticker, or cryptocurrency pair to display',
            name: 'Symbol',
        },
    },
};
const dataTickerSingleSparkline: ExampleProps = {
    url: 'http://127.0.0.1:8080/ticker-single-sparkline',
    params: {
        period: {
            type: 'string',
            default: '24h',
            description: 'Time period for price change calculation',
            enum: ['1h', '24h', '7d', '30d'],
            name: 'Time Period',
        },
        pair: {
            type: 'string',
            default: 'BTC-USD',
            description: 'Financial symbol, stock ticker, or cryptocurrency pair to display',
            name: 'Symbol',
        },
    },
};
const dataWeather: ExampleProps = {
    url: 'http://127.0.0.1:8080/weather',
    params: {
        location: {
            type: 'string',
            default: 'Prague',
            description: 'City name for weather forecast',
            name: 'Location',
        },
    },
};

export function Debug() {
    return <Example {...dataDebug} />;
}
export function ExchangeRate() {
    return <Example {...dataExchangeRate} />;
}
export function Nameday() {
    return <Example {...dataNameday} />;
}
export function TickerList() {
    return <Example {...dataTickerList} />;
}
export function TickerCandle() {
    return <Example {...dataTickerCandle} />;
}
export function TickerSingleSparkline() {
    return <Example {...dataTickerSingleSparkline} />;
}
export function Weather() {
    return <Example {...dataWeather} />;
}
