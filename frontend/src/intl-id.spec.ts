// Copyright (C) 2026  Braiins Forge s.r.o.
//
// This program is free software: you can redistribute it and/or modify
// it under the terms of the GNU General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.
//
// This program is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
// GNU General Public License for more details.
//
// You should have received a copy of the GNU General Public License
// along with this program.  If not, see <https://www.gnu.org/licenses/>.
//
// Braiins Systems s.r.o. and Braiins Forge s.r.o. each reserve the right
// to grant any party a license to this program, or any part thereof,
// under any terms, and such a grant shall be considered distinct from
// the grant above.

import { expect, test } from '@rstest/core';
import { createIntl } from 'react-intl';

// The id `formatjs extract` gives the message below;
// `bmc-intl/tools/src/formatjs.rs` holds the CLI to it.
const EXTRACTED_ID = '64/ZPeJ+Sp';

test('the build hashes a message to the id the CLI extracts', () => {
    const intl = createIntl({ locale: 'zh', defaultLocale: 'en', messages: { [EXTRACTED_ID]: '探针' } });
    const text = intl.formatMessage({ defaultMessage: 'Probe', description: 'Fixture for the message id hash' });
    expect(text).toBe('探针');
});
