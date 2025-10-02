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

    constructor(props: Props, context: AppContextType) {
        super(props);

        const { currentlyPlaying } = context.device.sound;
        this.state = { isPlaying: currentlyPlaying?.id === props.sound.id };
    }
    componentDidUpdate() {
        const { sound } = this.props;
        const { isPlaying } = this.state;
        const { currentlyPlaying } = this.context.device.sound;

        // Something outside played a sound, but we are not marked as playing
        if (currentlyPlaying?.id === sound.id && !isPlaying) this.setState({ isPlaying: true });
        // We think we are playing a sound, but context says otherwise
        else if (isPlaying && !currentlyPlaying) this.setState({ isPlaying: false });
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
            msg ||= intl.formatMessage({ defaultMessage: `Failed to play the sound ${sound.name}` });
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
