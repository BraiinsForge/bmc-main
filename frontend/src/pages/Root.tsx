import { Component } from 'react';
import { debounce } from 'es-toolkit';
import { Outlet, useNavigate, type NavigateFunction, useLocation } from 'react-router';

// App
import { URLS } from '@/constants';
import { useStore } from '@/store';
import AppContext, {
    getAppContextDefault,
    type AppContextType,
    type NotificationType,
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
    type: NotificationType;
    text: string;
}

interface State {
    notifications: ReadonlyArray<NotificationItem>;
    confirmation: null | Confirmation;
}
const getInitialState = (): State => ({
    notifications: [],
    confirmation: null,
});

class View extends Component<Props, State> {
    readonly state = getInitialState();

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
    #notificationHide = (id: Notification['id']): void => {
        this.setState({ notifications: this.state.notifications.filter(x => x.id !== id) });
    };
    #notificationClear = (_?: any): void => this.setState({ notifications: [] });
    #notify = (type: Notification['type'], text: Notification['text'], duration?: number): void => {
        const id: number = this.#notificationLastID++;
        const newItem = {
            id,
            kind: type,
            content: text,
        } satisfies NotificationItem;
        this.setState(s => ({ notifications: [...s.notifications, newItem] }));

        if (duration != null && Number.isInteger(duration)) {
            setTimeout(() => this.#notificationHide(id), duration * 3e3);
        }
    };

    #appContextValue = Object.assign({}, getAppContextDefault(), {
        notify: Object.assign(
            (type: Notification['type'], message: Notification['text'], timeoutSeconds?: number) => {
                this.#notify(type, message, timeoutSeconds);
            },
            { clear: this.#notificationClear },
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
    });

    render() {
        const { notifications } = this.state;

        return (
            <AppContext value={this.#appContextValue}>
                <Notifications
                    top={12}
                    items={notifications}
                    onHide={x => this.#notificationHide(x.id)}
                    onClear={this.#notificationClear}
                />

                {this.#confirmRender()}

                <Outlet />
            </AppContext>
        );
    }
}

export default function () {
    const { pathname } = useLocation();
    const navigate = useNavigate();
    const isRootPath: boolean = pathname === '/';
    const isAuthenticated = useStore(x => x.state.sessionInfo.isAuthenticated);
    return <View navigate={navigate} pathname={pathname} isAuthenticated={isAuthenticated} isRootPath={isRootPath} />;
}
