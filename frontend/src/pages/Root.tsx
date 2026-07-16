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
import { debounce } from 'es-toolkit';
import { Outlet, useNavigate, type NavigateFunction, useLocation } from 'react-router';

// Libs
import { setState } from '@/lib/react';
import { Toaster } from '@/lib/toast';

// App
import * as pb from '@/proto';
import { URLS } from '@/constants';
import { useStore } from '@/store';
import AppContext, { getAppContextDefault, type AppContextType, type ConfirmationDescriptor } from '@/context';

// Components
import { Modal } from '@/components';

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

interface State {
    confirmation: null | Confirmation;
    appContextValue: AppContextType;
}

class View extends Component<Props, State> {
    constructor(props: Props) {
        super(props);
        this.state = {
            confirmation: null,
            appContextValue: Object.assign({}, getAppContextDefault(), {
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
                children={<div style={{ textWrap: 'balance' }} children={message} />}
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
        const { appContextValue } = this.state;

        return (
            <AppContext value={appContextValue}>
                <Toaster position="top-right" visibleToasts={3} duration={4} />

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
