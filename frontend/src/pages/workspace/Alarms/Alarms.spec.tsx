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

import { afterEach, beforeEach, describe, test, expect, rstest } from '@rstest/core';
import { act, cleanup, fireEvent, render, screen, within } from '@testing-library/react/pure';
import { MemoryRouter } from 'react-router';
import { RawIntlProvider } from 'react-intl';
import { HelmetProvider } from '@dr.pogodin/react-helmet';
import { ConnectError, Code } from '@connectrpc/connect';

import AlarmsPage from './Alarms';
import * as pb from '@/proto';
import { mocks } from '@/proto/transport';
import { store } from '@/store';
import { deckCapabilities } from '@/pages/workspace/Display/capabilities.fixture';
import { Toaster, toast } from '@/lib/toast';
import { intlFor } from '@/lib/intl';
import type { ServiceMocks } from '@/lib/proto';

const advance = async (ms: number): Promise<void> => {
    await act(async () => {
        await rstest.advanceTimersByTimeAsync(ms);
    });
};

// `page: false` leaves the page but keeps this Toaster; a remounted one shows no toast.
function pageTree(page = true) {
    return (
        <HelmetProvider>
            <RawIntlProvider value={intlFor({ tag: 'en' })}>
                <MemoryRouter>
                    {page && <AlarmsPage />}
                    <Toaster />
                </MemoryRouter>
            </RawIntlProvider>
        </HelmetProvider>
    );
}

const alarm = (name: string) => pb.create(pb.AlarmSchema, { id: 'a1', name, time: '14:30', enabled: true });
const settings = (timeFormat: pb.TimeFormat) => pb.create(pb.GeneralSettingsDataResponseSchema, { timeFormat });

type ReadGeneralSettings = ServiceMocks<typeof pb.services.ConfigurationService>['getGeneralSettingsData'];
function answerGeneralSettings(read: ReadGeneralSettings): void {
    mocks.service(pb.services.ConfigurationService, {
        getGeneralSettingsData: read,
    } as ServiceMocks<typeof pb.services.ConfigurationService>);
}

/** Each load lists the next of `lists`, the last one repeating. */
function answerAlarms(...lists: pb.Alarm[][]): void {
    let loads = 0;
    rstest.spyOn(pb.rpc.alarm, 'listAlarms').mockImplementation(async () => {
        const alarms = lists[Math.min(loads, lists.length - 1)];
        loads += 1;
        return pb.create(pb.ListAlarmsResponseSchema, { alarms });
    });
    rstest.spyOn(pb.rpc.config, 'listSounds').mockResolvedValue(pb.create(pb.ListSoundsResponseSchema));
    rstest.spyOn(pb.rpc.alarm, 'getAlarmInfo').mockResolvedValue(pb.create(pb.AlarmInfoResponseSchema));
    rstest.spyOn(pb.rpc.alarm, 'setAlarmEnabled').mockResolvedValue(pb.create(pb.EmptySchema));
}

beforeEach(() => {
    rstest.useFakeTimers();
    store.setHardwareCapabilities(deckCapabilities());
});

afterEach(() => {
    cleanup();
    mocks.clear();
    rstest.restoreAllMocks();
    rstest.useRealTimers();
    store.setHardwareCapabilities(null);
});

describe('Alarms', () => {
    test('a reload overtaken by a newer one does not overwrite its list', async () => {
        answerAlarms([alarm('first')], [alarm('stale')], [alarm('fresh')]);
        let reads = 0;
        answerGeneralSettings(({ conf }) => {
            reads += 1;
            // The second load's lists arrive at once and its read lags,
            // so the third load answers first and the second resolves after it.
            if (reads === 2) conf({ delay: 1_000 });
            return settings(pb.TimeFormat.TIME_FORMAT_24_HOUR);
        });
        render(pageTree());
        await advance(0);
        expect(screen.getByText('first')).toBeTruthy();

        const toggle = within(screen.getByRole('table')).getByRole('switch');
        // Each toggle reloads once its debounce is past.
        fireEvent.click(toggle);
        await advance(300);
        fireEvent.click(toggle);
        await advance(300);
        await advance(2_000);

        expect(screen.getByText('fresh')).toBeTruthy();
    });

    test('a reload overtaken by a newer one that fails reports the failure once', async () => {
        answerAlarms([alarm('first')]);
        let reads = 0;
        answerGeneralSettings(({ conf }) => {
            reads += 1;
            if (reads === 2) conf({ delay: 1_000 });
            if (reads === 3) throw new ConnectError('unreachable', Code.Unavailable);
            return settings(pb.TimeFormat.TIME_FORMAT_24_HOUR);
        });
        render(pageTree());
        await advance(0);

        const toggle = within(screen.getByRole('table')).getByRole('switch');
        fireEvent.click(toggle);
        await advance(300);
        fireEvent.click(toggle);
        await advance(300);
        await advance(2_000);

        expect(reads, 'the third load overtook the second').toBe(3);
        expect(screen.getAllByText(/unreachable/)).toHaveLength(1);
    });

    test('a load that fails after the page is left raises no toast on the next page', async () => {
        answerAlarms([]);
        rstest
            .spyOn(pb.rpc.config, 'getGeneralSettingsData')
            .mockImplementation(
                () =>
                    new Promise((_, reject) =>
                        setTimeout(() => reject(new ConnectError('unreachable', Code.Unavailable)), 1_000),
                    ),
            );
        const view = render(pageTree());
        await advance(0);

        view.rerender(pageTree(false));
        await advance(2_000);

        expect(screen.queryByText(/unreachable/)).toBeNull();
    });

    test('a toggle that loses the device reports it in the catalog wording', async () => {
        answerAlarms([alarm('wake')]);
        answerGeneralSettings(() => settings(pb.TimeFormat.TIME_FORMAT_24_HOUR));
        rstest
            .spyOn(pb.rpc.alarm, 'setAlarmEnabled')
            .mockRejectedValue(new ConnectError('Failed to fetch', Code.Unavailable));
        render(pageTree());
        await advance(0);

        fireEvent.click(within(screen.getByRole('table')).getByRole('switch'));
        await advance(0);

        expect(screen.getByText('Failed to toggle alarm!')).toBeTruthy();
        expect(screen.queryByText(/Failed to fetch/)).toBeNull();
    });

    test.each([
        ['lost', 'drops', 0, false],
        ['still valid', 'shows', 1, true],
    ])(
        'a toggle the device refuses, its session %s, %s the failure toast',
        async (_session, _outcome, toasts, stillValid) => {
            answerAlarms([alarm('wake')]);
            answerGeneralSettings(() => settings(pb.TimeFormat.TIME_FORMAT_24_HOUR));
            rstest
                .spyOn(pb.rpc.alarm, 'setAlarmEnabled')
                .mockRejectedValue(new ConnectError('no session', Code.Unauthenticated));
            const recheckSession = rstest
                .spyOn(store, 'recheckSession')
                .mockImplementation(async () => toast.releaseErrors(stillValid));
            render(pageTree());
            await advance(0);

            fireEvent.click(within(screen.getByRole('table')).getByRole('switch'));
            await advance(0);

            expect(recheckSession).toHaveBeenCalledTimes(1);
            expect(screen.queryAllByText('Failed to toggle alarm!')).toHaveLength(toasts);
        },
    );

    test("each alarm's switch is named by its time and still shows its state", async () => {
        answerAlarms([alarm('wake')]);
        answerGeneralSettings(() => settings(pb.TimeFormat.TIME_FORMAT_24_HOUR));
        render(pageTree());
        await advance(0);

        const row = within(screen.getByRole('table'));
        expect(row.getByRole('switch', { name: 'Alarm at 14:30' })).toBeTruthy();
        expect(row.getByText('On')).toBeTruthy();
    });

    test('the alarm times follow a later change of the device clock', async () => {
        answerAlarms([alarm('wake')]);
        let held = settings(pb.TimeFormat.TIME_FORMAT_24_HOUR);
        answerGeneralSettings(() => held);
        render(pageTree());
        await advance(0);
        expect(screen.getByText('14:30')).toBeTruthy();

        held = settings(pb.TimeFormat.TIME_FORMAT_12_HOUR);
        void store.loadFormats();
        await advance(0);

        expect(screen.getByText('2:30 PM')).toBeTruthy();
    });
});
