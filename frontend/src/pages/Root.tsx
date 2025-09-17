import { Component } from 'react';
import { debounce } from 'es-toolkit';
import { Outlet, useNavigate, type NavigateFunction, useLocation } from 'react-router';

// Libs
import { setState } from '@/lib/react';

// App
import * as pb from '@/proto';
import { URLS } from '@/constants';
import { useStore } from '@/store';
import AppContext, {
    getAppContextDefault,
    type AppContextType,
    type NotifyFunction,
    type NotificationExtra,
    type ConfirmationDescriptor,
} from '@/context';

// Components
import { Modal, Notifications, type NotificationItem } from '@/components';

// Styles
import '@/styles/carbon/carbon.global.scss';

interface Props {
    pathname: string;
    navigate: NavigateFunction;
    isRootPath: boolean;
    isAuthenticated: null | boolean;
}

interface Confirmation extends ConfirmationDescriptor {
    id: number;
    confirm(): void;
    cancel(): void;
}
interface Notification {
    id: string | number;
    externalID: null | NotificationExtra['id'];
    data: NotificationItem;
}

interface State {
    notifications: Notification[];
    confirmation: null | Confirmation;
    appContextValue: AppContextType;
}

class View extends Component<Props, State> {
    constructor(props: Props) {
        super(props);
        this.state = {
            notifications: [],
            confirmation: null,
            appContextValue: Object.assign({}, getAppContextDefault(), {
                notify: Object.assign(
                    ((type, message, extra) => this.#notify(type, message, extra)) as NotifyFunction,
                    {
                        clear: this.#notificationClearExternal,
                    },
                ) as AppContextType['notify'],
                confirm: (conf => {
                    return this.#confirm({
                        size: conf.size,
                        danger: conf.danger,

                        title: conf.title,
                        message: conf.message,

                        confirmLabel: conf.confirmLabel,
                        cancelLabel: conf.cancelLabel,
                    });
                }) as AppContextType['confirm'],
                device: {
                    sound: {
                        currentlyPlaying: null,
                        play: this.#soundPlay,
                        stop: this.#soundStop,
                    },
                },
            } satisfies AppContextType),
        };
    }
    componentDidMount = () => this.#mount();
    componentDidUpdate(prevProps: Readonly<Props>) {
        const { isRootPath, isAuthenticated } = this.props;
        // Since there is nothing usefull on the roo path,
        // we have to always redirect it to "somthing"
        if (prevProps.isAuthenticated !== isAuthenticated || isRootPath) this.#maybeRedirect();
    }

    /**
     * Mount methods are extracted into their own debounced method to avoid problems arising from react's double render.
     * Instead of needless setup/teardown multiple times we'll avoid the problem by debouncing the mount method.
     * It does introduce a slight delay, but it's much more elegant and ultimately performant.
     */
    #mount = debounce(async () => {
        this.#maybeRedirect();
    }, 150);
    #maybeRedirect = async (): Promise<void> => {
        const { navigate, pathname, isAuthenticated, isRootPath } = this.props;
        const { login } = URLS.auth;

        const isPublicPage: boolean = Object.values(URLS.auth).some(x => pathname.startsWith(x));

        // Redirects based on authentication status
        //  - redirect to dashboard "from login / signup" if switched to authenticated
        //  - redirect to login if switched to UNauthenticated
        if (isAuthenticated === true && (isPublicPage || isRootPath)) return navigate(URLS.defaultScreen);
        if (isAuthenticated === false && !isPublicPage) return navigate(login);
    };

    //
    // Confirmation dialog
    //

    #confirmLastID: Confirmation['id'] = 0;
    #confirmQueue: Map<number, Confirmation> = new Map();
    #confirmRender(): ReactNode {
        const queue = this.#confirmQueue;
        const firstEntry: undefined | Confirmation = queue.values().next().value;

        const size = firstEntry?.size ?? 'xs';
        const title = firstEntry?.title as string;
        const message = firstEntry?.message as string;
        const labelConfirm = firstEntry?.confirmLabel ?? 'Confirm';
        const labelCancel = firstEntry?.cancelLabel ?? 'Cancel';

        const confirm = () => firstEntry?.confirm?.();
        const cancel = () => firstEntry?.cancel();

        return (
            <Modal
                id="confirmation-modal"
                style={{ zIndex: 9e6 }}
                open={!!firstEntry}
                size={size}
                modalHeading={title}
                // Submit
                primaryButtonText={labelConfirm}
                onRequestSubmit={confirm}
                // Cancel
                secondaryButtonText={labelCancel}
                onSecondarySubmit={cancel}
                // Close button
                onRequestClose={cancel}
                children={message}
                danger={firstEntry?.danger}
            />
        );
    }
    #confirm: AppContextType['confirm'] = conf => {
        const id = this.#confirmLastID;
        this.#confirmLastID += 1;
        const handleRemove = () => {
            this.#confirmQueue.delete(id);
            this.forceUpdate();
        };

        return new Promise(resolve => {
            this.#confirmQueue.set(id, {
                id,
                ...conf,
                confirm() {
                    handleRemove();
                    resolve(true);
                },
                cancel() {
                    handleRemove();
                    resolve(false);
                },
            });
            this.forceUpdate();
        });
    };

    //
    // Notifications
    //

    #notificationLastID: number = 0;
    #notificationClearInternal = (id: Notification['id']): void => {
        this.setState({ notifications: this.state.notifications.filter(x => x.id !== id) });
    };
    #notificationClearExternal = (externalID?: NotificationExtra['id']): void => {
        this.setState(s => ({
            notifications: externalID != null ? s.notifications.filter(x => x.externalID !== externalID) : [],
        }));
    };
    #notificationClearExternalAll = (): void => this.#notificationClearExternal();
    #notify: NotifyFunction = (type, text, extra): void => {
        const externalID = extra?.id ?? null;
        const timeoutSeconds = extra?.timeoutSeconds;

        this.#notificationLastID += +1;
        const id: number = this.#notificationLastID;

        let currentList = this.state.notifications.slice(0);
        const data: NotificationItem = { id, kind: type, content: text, counter: null };

        if (externalID != null) {
            const existingItem = currentList.find(x => x.externalID === externalID);
            currentList = currentList.filter(x => x.externalID !== externalID);
            if (existingItem) data.counter = (existingItem.data.counter ?? 0) + 1;
        }

        this.setState({
            notifications: [...currentList, { id, externalID, data }],
        });

        if (timeoutSeconds != null && Number.isFinite(timeoutSeconds)) {
            setTimeout(() => this.#notificationClearInternal(id), timeoutSeconds * 3e3);
        }
    };

    private soundPlayAbort = pb.abort.get();
    #soundSetPlaying = (sound: null | pb.SoundInfo): Promise<void> => {
        return setState(this, s => ({
            appContextValue: {
                ...s.appContextValue,
                device: {
                    ...s.appContextValue.device,
                    sound: {
                        ...s.appContextValue.device.sound,
                        currentlyPlaying: sound,
                    },
                },
            },
        }));
    };
    #soundPlay = async (sound: pb.SoundInfo, signal: AbortSignal): Promise<void> => {
        const { signal: abortSignal } = this.soundPlayAbort.replace().attach(signal);
        await this.#soundSetPlaying(sound);

        try {
            await pb.rpc.config.playSound({ soundId: sound.id }, { signal: abortSignal });
        } catch ($) {
            // Abort error is swallowed here, rest is propagated down to the caller
            if (pb.abort.is($)) return console.log('Aborted sound playback', sound);
            throw $;
        } finally {
            /**
             * To prevent a race condition in setting the currently playing sound,
             * we have to check whether we are reseting a known state that we have set.
             *
             * This is because the finally block runs asynchronously
             * AFTER the new triggers the abort signal and save the sound info.
             */
            const { currentlyPlaying } = this.state.appContextValue.device.sound;
            if (currentlyPlaying?.id === sound.id) this.#soundSetPlaying(null);
        }
    };
    #soundStop = async (): Promise<void> => {
        this.soundPlayAbort.replace();
    };

    render() {
        const { notifications, appContextValue } = this.state;

        return (
            <AppContext value={appContextValue}>
                <Notifications
                    top={12}
                    items={notifications.map(x => x.data)}
                    onHide={x => this.#notificationClearInternal(x.id)}
                    onClear={this.#notificationClearExternalAll}
                />

                {this.#confirmRender()}

                <Outlet />
            </AppContext>
        );
    }
}

export default function Root() {
    const { pathname } = useLocation();
    const navigate = useNavigate();
    const isRootPath: boolean = pathname === '/';
    const isAuthenticated = useStore(x => x.state.sessionInfo.isAuthenticated);
    return <View navigate={navigate} pathname={pathname} isAuthenticated={isAuthenticated} isRootPath={isRootPath} />;
}
