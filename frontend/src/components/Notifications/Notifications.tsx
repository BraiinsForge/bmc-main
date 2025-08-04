import { Component, createRef, Fragment } from 'react';
import { injectIntl, type IntlShape } from 'react-intl';

import gsap from 'gsap';
import { cloneDeep } from 'es-toolkit';
import isEqual from 'react-fast-compare';

// Components
import { Html } from '@/components/format/Html';
import { Button } from '@/components/Button';
import { ToastNotification, type ToastNotificationProps } from '@carbon/react';
import { ListChecked } from '@carbon/react/icons';

// CSS
import css from './Notifications.scss';
import cn from 'clsx';

type ID = StrNum;

export interface NotificationItem extends Pick<ToastNotificationProps, 'kind'> {
    id: ID;
    title?: ReactNode;
    content: NonNullable<ReactNode>;
    onClick?(): void;
    counter?: null | number;
}
type Items = ReadonlyArray<NotificationItem>;

export interface NotificationsProps {
    top?: number;
    items: Items;

    onHide(x: NotificationItem): void;
    onClear?(): void;

    className?: string;
    style?: CSSProperties;
}
interface Props extends NotificationsProps {
    intl: IntlShape;
}

const margin = 8;
const cloneItems = <T extends Items>(d: Maybe<T>): T => cloneDeep<T>(d as T);

const tweenConf = Object.freeze({ duration: 0.5, ease: 'Power1.easeOut' });
const getRemovalTween = (node: HTMLElement) => {
    let x = 300;
    if (document.documentElement.getAttribute('dir') === 'rtl') x *= -1;
    return gsap.to(node, { x, alpha: 0.5, ...tweenConf });
};

class NotificationsComponent extends Component<Props> {
    #ref = createRef<HTMLDivElement>();

    #lockedUpdates: boolean = false;
    #pendingUpdate: null | Items = null;
    #lock = (): void => {
        this.#lockedUpdates = true;
    };
    #unlock = (): void => {
        // Clone in pending updates
        if (this.#pendingUpdate) {
            this.#clone(this.#pendingUpdate);
            this.#pendingUpdate = null;
        }

        // Unlock
        this.#lockedUpdates = false;
    };

    // Clone data to local storage (and previous state as well)
    #clone = (d: Items, render?: boolean): void => {
        this.#itemsPrev = cloneItems(this.#itemsCurr);
        this.#itemsCurr = cloneItems(d).toReversed();

        const done = () =>
            this.forceUpdate(() => this.#tween(undefined, this.#itemsPrev.length > this.#itemsCurr.length));

        const root = this.#ref.current;
        if (render && root) {
            const children = root.children;

            const idsCurr = new Set<ID>(this.#itemsCurr.map(x => x.id));
            const removed = new Set<number>();
            this.#itemsPrev.forEach((x, index) => {
                if (!idsCurr.has(x.id)) removed.add(index);
            });

            if (removed.size) {
                const tl = gsap.timeline({ paused: true, onComplete: done });
                removed.forEach(i => tl.add(getRemovalTween(children[i] as HTMLElement), 0));
                tl.play(0);
            } else {
                done();
            }
        }
    };

    // We'll keep two copies of data so that we ensure stability during animations
    // and keep the ability to detect updates between renders
    #itemsCurr: Items = [];
    #itemsPrev: Items = [];

    // Timeline reference is kept so that it can be
    // killed on onmount and before sequential updates
    #timeline?: GSAPTimeline;

    constructor(props: Props) {
        super(props);
        this.#itemsCurr = cloneItems(props.items).toReversed();
        this.#itemsPrev = cloneItems(this.#itemsCurr);
    }
    shouldComponentUpdate = () => !this.#lockedUpdates;
    componentDidUpdate(prevProps: NotificationsProps) {
        if (!isEqual(this.props.items, prevProps.items)) {
            if (this.#lockedUpdates) this.#pendingUpdate = cloneItems(this.props.items);
            else this.#clone(this.props.items, true);
        }
    }
    componentDidMount = () => this.#tween();
    componentWillUnmount = () => this.#timeline?.kill();

    /**
     * Method that handles animation of individual notification messages.
     *
     * It has a special animation for the newest item detected as new,
     * but it has a false positive when called after item removal,
     * so we have to be signaled in that case & skip the check.
     */
    #tween = (onComplete?: AnyFunction<void>, isRemovalSync?: boolean): void => {
        // Setting progress to one first makes sure that the "onComplete" callback
        // is called before killing the timeline.
        // It will not be called again if the timeline was already completed.
        this.#timeline?.progress(1).kill();

        // I was observed that Timeline's onComplete callback does not get called when the timeline is empty.
        // To work around this issue, we'll call it manually if there are no children (hence empty timeline).
        const children = (this.#ref.current?.children ?? []) as ArrayLike<HTMLDivElement>;
        if (!children.length) {
            onComplete?.();
            return;
        }

        const timeline: GSAPTimeline = gsap.timeline({ pause: true, onComplete });
        this.#timeline = timeline;

        // Local data is reversed, so the last item is at 0
        const firstCurr = this.#itemsCurr[0] as Maybe<NotificationItem>;
        const firstPrev = this.#itemsPrev[0] as Maybe<NotificationItem>;
        const hasNew: boolean = !isRemovalSync && firstCurr?.id !== firstPrev?.id;

        let offset: number = 0;
        let lastOffset: number = 0;
        Array.from<HTMLDivElement>(children).forEach((ref, i) => {
            if (!ref.style) gsap.to(ref, { duration: 0, x: 0, y: lastOffset, alpha: 1 });

            const height = ref.getBoundingClientRect().height;
            let tween: gsap.core.Tween;
            if (i === 0 && hasNew) {
                tween = gsap.fromTo(
                    ref,
                    { y: offset - height * 2, alpha: 0.5 },
                    { x: 0, y: offset, alpha: 1, ...tweenConf },
                );
            } else {
                tween = gsap.to(ref, { x: 0, y: offset, alpha: 1, ...tweenConf });
            }
            timeline.add(tween, 0);

            lastOffset = offset;
            offset += height + margin;
        });

        timeline.play(0);
    };

    // Remove the item from current queue, sync data & signalise parent
    #handleClose = (index: number, item: NotificationItem): void => {
        const node = this.#ref.current?.children[index] as Maybe<HTMLElement>;
        if (!node) return;
        this.#lock();

        // remove the item so that it isn't immediately re-rendered
        this.#itemsCurr = this.#itemsCurr.filter(x => x.id !== item.id);
        this.#itemsPrev = this.#itemsPrev.filter(x => x.id !== item.id);

        // Notify parent of removal
        this.props.onHide(item);
        if (item.onClick) item.onClick();

        gsap.timeline({
            paused: true,
            onComplete: () => {
                // render to update DOM
                this.forceUpdate(() => {
                    // tween the rest of the items to their new positions
                    // and unlock props updates (should sync the states)
                    this.#tween(this.#unlock, true);
                });
            },
        })
            .add(getRemovalTween(node))
            .play(0);
    };

    render() {
        const { top, className, onClear, intl } = this.props;

        const style = { ...this.props.style, top: (top || 0) + 16 };
        const items = this.#itemsCurr.map((item, i) => {
            const { id, kind = 'info', title, content, counter } = item;
            return (
                <div key={id} className={cn(css.item, css[kind])}>
                    <ToastNotification
                        kind={kind}
                        // @ts-expect-error: They enforce for A11Y reasons, but do not give a flying fuck about that
                        // https://github.com/carbon-design-system/carbon/pull/16911#pullrequestreview-2161008750
                        title={title || ''}
                        children={
                            <Fragment>
                                {typeof content === 'string' ? <Html children={content} /> : content}
                                {!!counter && <span className={css.counter}>{counter}&times;</span>}
                            </Fragment>
                        }
                        onClose={() => {
                            this.#handleClose(i, item);
                            // Returning false prevents hiding in the carbon component
                            return false;
                        }}
                    />
                </div>
            );
        });

        return (
            <div ref={this.#ref} style={style} className={cn(css.root, className)}>
                {items}
                {typeof onClear === 'function' && items.length > 1 ? (
                    <Button
                        size="md"
                        kind="secondary"
                        icon={ListChecked}
                        onClick={onClear}
                        className={css.clearAllButton}
                        children={intl.formatMessage({ defaultMessage: 'Close All' })}
                    />
                ) : null}
            </div>
        );
    }
}

export const Notifications = injectIntl<'intl', Props>(NotificationsComponent, { forwardRef: true });
