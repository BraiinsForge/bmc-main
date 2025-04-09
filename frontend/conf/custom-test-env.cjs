const DomEnvironment = require('jest-environment-jsdom').default;
const { TextEncoder, TextDecoder } = require('node:util');

/**
 * @see https://jestjs.io/docs/configuration#testenvironment-string
 * @see https://stackoverflow.com/a/57713960/2179323
 */
module.exports = class CustomTestEnvironment extends DomEnvironment {
    async setup() {
        await super.setup();
        if (!this.global.TextEncoder) this.global.TextEncoder = TextEncoder;
        if (!this.global.TextDecoder) this.global.TextDecoder = TextDecoder;
        if (!this.global.setImmediate) this.global.setImmediate = (fn, ...args) => global.setTimeout(fn, 0, ...args);

        // Set default timezone for timezone sensitive tests
        process.env.TZ = 'UTC';
    }
};
