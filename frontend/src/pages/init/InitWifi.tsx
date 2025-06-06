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
            this.setState({ errors: pb.collectAllErrors($) ?? ['Failed to load WiFi networks!'] });
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
            this.setState({ errors: pb.collectAllErrors($) ?? ['Failed to set WiFi!'] });
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
