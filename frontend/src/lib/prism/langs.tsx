import Prism from 'prismjs';

export { Prism };

Prism.manual = true;
global.Prism = Prism;

// Custom "language" called "URL" to beautify
// examples of URLs with some dynamic segments

function prism_log() {
    Prism.languages.log = {
        info: /\binfo /i,
        error: /\berr(or)? /i,
        warning: /\bwarn(ing) ?/,
        date: /(jan|feb|mar|apr|may|jun|jul|aug|sep|oct|nov|dec) \d{2} \d{2}:\d{2}:\d{2}(\.\d{3})?/i,
    };
}

function prism_url() {
    Prism.languages.url = {
        path: {
            pattern: /(\/)[\w.]+(?=\/)/,
            lookbehind: true,
        },
        protocol: /^.*?:\/{2}/,
        variable: {
            pattern: /([&?])\w+(?==)/,
            lookbehind: true,
        },
        placeholder: /\[.*?]/,
        value: {
            pattern: /(=)[\w-_]+(?=&|$|#)?/,
            lookbehind: true,
        },
        operator: /[:@?&=]/,
        punctuation: /[/]/,
    };
}

function prism_regexp() {
    Prism.languages.regexp = {
        container: [
            // Start
            /^\//,
            // End + flags
            /\/[gimsxU]*\s*$/,
        ],
        anchor: [
            { pattern: /(^|[^\\])\^/, lookbehind: true },
            { pattern: /(^|[^\\])\$/, lookbehind: true },
        ],
        class: /\\\w/, // \s \w \n …
        escaped: /\\[\\/[\]().?^$]/,
        quantifier: [
            // +, *
            /[+*]/,
            // {3}, {4,}
            /\{\d,?}/,
            // {3,4}
            /\{\d+,\d+}/,
        ],
    };

    const positiveLookahead = /\(\?=.*?\)/.source;
    const negativeLookahead = /\(\?!.*?\)/.source;
    const positiveLookbehind = /\(\?<=.*?\)/.source;
    const negativeLookbehind = /\(\?<!.*?\)/.source;
    const nonCapturingGroup = /\(\?:?.*?\)/.source;
    Prism.languages.insertBefore('regexp', 'anchor', {
        set: {
            pattern: /\[.*?]/,
            inside: {
                class: Prism.languages.regexp.class,
                escaped: Prism.languages.regexp.escaped,
                dash: {
                    pattern: /(\w)-(?=\w)/,
                    lookbehind: true,
                },
                start: /^\[/,
                end: /]$/,
            },
        },
        group: {
            pattern: new RegExp(
                `${positiveLookahead}|${negativeLookahead}|${positiveLookbehind}|${negativeLookbehind}|${nonCapturingGroup}`,
            ),
            inside: {
                // set: Prism.languages.regexp.set,
                class: Prism.languages.regexp.class,
                escaped: Prism.languages.regexp.escaped,
                quantifier: Prism.languages.regexp.quantifier,
            },
        },
    });
}

function prism_cron() {
    Prism.languages.cron = {
        times: {
            //                           month              week
            //         minutes  hours    day      month     day
            pattern: /^([^\s]+)[\s\t]+([^\s]+)[\s\t]+([^\s]+)[\s\t]+([^\s]+)[\s\t]+([^\s]+)/gm,
            greedy: false,
            inside: {
                operator: /[,/-]/,
                star: /[*]/,
                value: /\d+|mon|tue|wed|thu|fri|sat|sun/,
            },
        },
        command: {
            pattern: /.+$/m,
            greedy: false,
            inside: Prism.languages.bash,
        },
    };
}

[prism_log, prism_url, prism_regexp, prism_cron].forEach(fn => {
    try {
        fn();
    } catch (e) {
        console.log('An error has occured during loading of custom prism code', e);
    }
});
