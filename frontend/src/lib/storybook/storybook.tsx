import { createElement, useEffect, useState, type ReactElement, type ComponentType } from 'react';

// Libs
import type { StoryAnnotations, ArgsStoryFn, Renderer, Args } from '@storybook/types';

// Our libs
import { Loading } from '@/components';

// Styles
import css from './storybook.scss';

export type Story<Args> = ArgsStoryFn<Renderer, Args> & StoryAnnotations<Renderer, Args>;

export function withBackground(
    children: ReactNode,
    rootProps: { className?: string; style?: CSSProperties } = {},
): ReactElement {
    return (
        <div {...rootProps}>
            <div className={css.background} />
            {children}
        </div>
    );
}

/**
 * Ensure that `document.body` has a class name while the component is mounted.
 * Usefull for triggering different styles in the storybook modules
 * (for example, keeping a page background or canceling it).
 *
 * @example <><BodyClass className="demo" />{…}</>
 * @example <BodyClass className="demo" children={…} />
 */
export function BodyClass(props: { className: string; children?: ReactNode }) {
    useEffect(() => {
        document.body.classList.add(props.className);
        return () => document.body.classList.remove(props.className);
    }, [props.className]);
    return props.children ?? null;
}

interface DelayedProps {
    setup(): unknown | Promise<unknown>;
    render(key: string): ReactElement;
}
export function Delayed({ setup, render }: DelayedProps) {
    const [isReady, setReady] = useState(false);

    useEffect(() => {
        Promise.resolve(setup()).finally(() => setReady(true));
    }, [setup]);

    return isReady ? render(String(isReady)) : <Loading size={128} active cover />;
}

/**
 * Get the Figma "design" story parameter.
 * Usefull for the default export when the same design applies to all stories in the module.
 *
 * @example Usage:
 * export default {
 *     title: 'root/components',
 *     parameters: {
 *         design: figmaLink('https://www.figma.com/file/DEADBEEF/Foo-Bar?type=design&node-id=3087-32555'),
 *     },
 * };
 */
export function figmaDesign(url: string) {
    return { type: 'figma', url };
}

/**
 * Add the Figma "design" story parameter to the given story function.
 * Usefull for the named exports where each one might have it's separate design.
 *
 * @example Usage:
 * export function StoryFn() {…}
 * addFigmaDesign(StoryFn, 'https://www.figma.com/file/DEADBEEF/Foo-Bar?type=design&node-id=3087-32555');
 */
export function addFigmaDesign<TArgs extends Args = Args>(story: MaybeArray<Story<TArgs>>, url: string): void {
    const array = Array.isArray(story) ? story : [story];
    array.forEach(s => {
        s.parameters = { ...s.parameters, design: figmaDesign(url) };
    });
}

export type StoryAnnotate = {
    design?: string;
};
export function annotated<TArgs extends Args = Args>(subject: ComponentType<TArgs>, attrs: StoryAnnotate) {
    const res = (args => {
        return createElement(subject, args);
    }) as Story<TArgs>;

    if (attrs.design) addFigmaDesign(res, attrs.design);

    return res;
}

export type { Meta, StoryObj } from '@storybook/react';
export { action } from '@storybook/addon-actions';
