// Copyright (C) 2025  Braiins Systems s.r.o.
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

import { describe, test, expect } from '@rstest/core';

import { unloadGuard } from './unloadGuard';

// `true` when a guard cancelled the unload, i.e. the browser would ask first.
const leaveIsQuestioned = (): boolean => !window.dispatchEvent(new Event('beforeunload', { cancelable: true }));

describe('lib/unloadGuard', () => {
    test('one run finishing keeps guarding the page for a run still going', () => {
        const releaseUpgrade = unloadGuard.enable();
        const releaseInstall = unloadGuard.enable();

        releaseInstall();
        expect(leaveIsQuestioned()).toBe(true);

        releaseUpgrade();
        expect(leaveIsQuestioned()).toBe(false);
    });

    test('a run releasing twice does not release the hold of another', () => {
        const releaseUpgrade = unloadGuard.enable();
        const releaseInstall = unloadGuard.enable();

        releaseInstall();
        releaseInstall();
        expect(leaveIsQuestioned()).toBe(true);

        releaseUpgrade();
    });
});
