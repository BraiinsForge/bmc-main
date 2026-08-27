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

import { Component } from 'react';

// App, lib
import * as pb from '@/proto';
import { setState } from '@/lib/react';

// Components
import { InlineNotificationsGroup } from '@/components';
import { Welcome, WifiConnect } from './components';

// Styles
import '@/styles/carbon/carbon.global.scss';
import css from './Init.scss';

enum Stage {
    welcome = 'welcome',
    wifi = 'wifi',
}

interface State {
    stage: Stage;
    wifi: {
        isLoading: boolean;
        networks: pb.WifiNetwork[];
    };
    errors: null | string[];
}
const getInitialState = (): State => ({
    stage: Stage.welcome,
    wifi: {
        isLoading: false,
        networks: [],
    },
    errors: null,
});

export default class InitWifi extends Component<any, State> {
    readonly state = getInitialState();

    componentDidMount = () => this.#scanWifi();
    componentWillUnmount = () => pb.abort.all(this);

    private abortScanWifi = pb.abort.get();
    #scanWifi = async (): Promise<void> => {
        const { signal } = this.abortScanWifi.replace();

        await setState(this, s => ({ wifi: { ...s.wifi, isLoading: true } }));
        let networks: pb.WifiNetwork[] = [];

        try {
            const res = await pb.rpc.init.scanWifi({}, { signal });
            networks = res.networks;
        } catch ($) {
            if (pb.abort.is($)) return;
            this.setState({ errors: pb.collectAllErrors($) ?? ['Failed to load Wi-Fi networks!'] });
        }

        this.setState({ wifi: { isLoading: false, networks } });
    };

    #gotoWelcome = (): void => this.setState({ stage: Stage.welcome });
    #gotoWifi = (): void => this.setState({ stage: Stage.wifi });

    #wifiSelect = (x: pb.WifiNetwork): void => console.log(x);
    #wifiSubmit = async (data: pb.SetWifiRequest): Promise<boolean> => {
        try {
            await pb.rpc.init.setWifi(data);
            return true;
        } catch ($) {
            if (pb.abort.is($)) return false;
            this.setState({ errors: pb.collectAllErrors($) ?? ['Failed to set Wi-Fi!'] });
            return false;
        }
    };

    render() {
        const { stage, wifi, errors } = this.state;

        let content: ReactNode = null;
        switch (stage) {
            case Stage.welcome:
                content = <Welcome onNext={this.#gotoWifi} />;
                break;

            case Stage.wifi:
                content = (
                    <WifiConnect
                        networks={wifi.networks}
                        onSelect={this.#wifiSelect}
                        onReload={this.#scanWifi}
                        isLoading={wifi.isLoading}
                        onBack={this.#gotoWelcome}
                        onSubmit={this.#wifiSubmit}
                    />
                );
                break;
        }

        return (
            <div className={css.root}>
                <div className={css.inner}>
                    <InlineNotificationsGroup kind="error" theme="inverse" items={errors} stretch />
                    {content}
                </div>
            </div>
        );
    }
}
