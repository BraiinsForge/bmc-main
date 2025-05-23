// core-js must be imported in the entry module directly, not transitively in here!
import 'regenerator-runtime/runtime';
import 'abortcontroller-polyfill/dist/polyfill-patch-fetch';

import '@formatjs/intl-locale/polyfill';
import '@formatjs/intl-pluralrules/polyfill';
import '@formatjs/intl-pluralrules/locale-data/en';
import '@formatjs/intl-relativetimeformat/polyfill';
import '@formatjs/intl-relativetimeformat/locale-data/en';
