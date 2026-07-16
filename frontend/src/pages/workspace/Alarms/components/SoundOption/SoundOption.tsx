// Copyright (C) 2025  Braiins Systems s.r.o.
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

import { Component, type UIEvent } from 'react';
import { abort } from '@/lib/abort';
import { setState } from '@/lib/react';
import { useIntl, type IntlShape } from 'react-intl';
import { toast } from '@/lib/toast';

// App
import * as pb from '@/proto';
import AppContext, { type AppContextType } from '@/context';

// Components
import { SoundPlayIcon } from '../SoundPlayIcon';

// CSS
import cn from 'clsx';
import css from './SoundOption.scss';

export interface SoundOptionProps {
    sound: pb.SoundInfo;

    id?: string;
    style?: CSSProperties;
    className?: string;
}
interface Props extends SoundOptionProps {
    intl: IntlShape;
}

interface State {
    isPlaying: boolean;
}

class View extends Component<Props, State> {
    static contextType = AppContext;
    declare context: AppContextType;

    #lastKnownCurrentlyPlaying: null | pb.SoundInfo = null;
    constructor(props: Props, context: AppContextType) {
        super(props);

        const { currentlyPlaying } = context.device.sound;
        this.state = { isPlaying: currentlyPlaying?.id === props.sound.id };
        this.#lastKnownCurrentlyPlaying = currentlyPlaying;
    }

    componentDidUpdate() {
        const { sound } = this.props;
        const { currentlyPlaying } = this.context.device.sound;

        if (currentlyPlaying?.id !== this.#lastKnownCurrentlyPlaying?.id) {
            this.#lastKnownCurrentlyPlaying = currentlyPlaying;
            this.setState({ isPlaying: currentlyPlaying?.id === sound.id });
        }
    }
    componentWillUnmount = () => abort.all(this);

    private abortPlaying = abort.get();
    #play = async (): Promise<void> => {
        const { signal } = this.abortPlaying.replace();

        const { device } = this.context;
        const { sound, intl } = this.props;

        try {
            await setState(this, { isPlaying: true });
            await device.sound.play(sound, signal);
        } catch ($) {
            let msg = pb.collectAllErrorsAsFormattedList($);
            msg ||= intl.formatMessage({ defaultMessage: 'Failed to play the sound {name}' }, { name: sound.name });
            toast.error(msg);
        } finally {
            this.setState({ isPlaying: false });
        }
    };
    #stop = () => this.abortPlaying.replace();

    #click = (e: UIEvent): void => {
        e.stopPropagation();
        e.preventDefault();

        const { isPlaying } = this.state;
        if (isPlaying) this.#stop();
        else this.#play();
    };

    render() {
        const { id, style, className, sound } = this.props;
        const { isPlaying } = this.state;

        return (
            <div id={id} className={cn(css.root, className)} style={style}>
                <div className={css.label} children={sound.name} />
                <button
                    type="button"
                    className={css.playButton}
                    onClickCapture={this.#click}
                    children={<SoundPlayIcon isPlaying={isPlaying} />}
                />
            </div>
        );
    }
}

export function SoundOption(props: SoundOptionProps) {
    const intl = useIntl();
    return <View {...props} intl={intl} />;
}
