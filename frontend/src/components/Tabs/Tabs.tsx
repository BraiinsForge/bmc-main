import type { Ref } from 'react';

import cn from 'clsx';
import css from './Tabs.scss';

export type BadgeKind = 'primary' | 'secondary' | 'danger' | 'custom';
export type TabsPropsTab<K extends StrNum = string> = {
    key: K;
    label: ReactNode;
    content?: MaybeGetter<ReactNode>;
    disabled?: boolean;

    badge?: MaybeGetter<ReactNode>;
    badgeKind?: null | BadgeKind;

    // Styling
    className?: string;
    style?: CSSProperties;
};
export type TabsPropsTabs<K extends StrNum = string> = ReadonlyArray<TabsPropsTab<K>>;
export type TabsProps<K extends StrNum = string> = {
    tabs: TabsPropsTabs<K>;
    tabsExtraContent?: ReactNode;

    activeTab: K;
    onChange(k: K): void;

    children?: ReactNode;
    render?(k: K): ReactNode;

    className?: string;
    tabContentClassName?: string;
    tabContentRef?: Ref<HTMLDivElement>;

    darkHeader?: boolean;
    noPadding?: boolean;
    noWrap?: boolean;
    size?: null | 'dense';

    style?: CSSProperties;
    tabContentStyle?: CSSProperties;
};

export function Tabs<K extends StrNum = string>(props: TabsProps<K>) {
    const {
        // Content
        children,
        render,

        // Tabs
        tabs,
        tabsExtraContent,
        activeTab,
        onChange,
        tabContentRef,

        // Styling
        tabContentClassName,
        darkHeader,
        noPadding,
        noWrap,
        size,
        tabContentStyle,
        ...rest
    } = props;

    let tabContent: ReactNode = typeof render === 'function' ? render(activeTab) : children;
    const $tabs: ReactNode[] = tabs.map((tab, index) => {
        const {
            key,
            disabled,
            // Content
            label,
            content,
            // Badge
            badge,
            badgeKind,
            // Styling
            className,
            style,
        } = tab;

        let badgeContent = typeof badge === 'function' ? badge() : badge;
        if (badgeKind !== 'custom' && badgeContent != null) {
            badgeContent = <span className={cn(css.badge, css[badgeKind || 'primary'])} children={badgeContent} />;
        }

        const isActive = activeTab === key;
        if (isActive && content != null) tabContent = typeof content === 'function' ? content() : content;

        return (
            <button
                type="button"
                key={index}
                children={
                    <span className={css.tabContent}>
                        <span children={label} />
                        {badgeContent}
                    </span>
                }
                style={style}
                className={cn(css.tab, className, isActive && css.active, disabled && css.disabled)}
                onClick={() => {
                    if (isActive) return;
                    onChange(key);
                    // @ts-expect-error: Missing blur method in DOM api
                    document.activeElement?.blur();
                }}
                disabled={disabled}
            />
        );
    });
    if (tabsExtraContent != null) $tabs.push(tabsExtraContent);

    return tabs.length === 0 ? null : (
        <div {...rest} className={cn(noPadding && css.noPadding, rest.className)}>
            <div
                className={cn(css.tabs, darkHeader && css.darkHeader, noWrap && css.noWrap, size && css[size])}
                children={$tabs}
            />
            <div
                ref={tabContentRef}
                className={cn(css.content, tabContentClassName)}
                children={tabContent}
                style={tabContentStyle}
            />
        </div>
    );
}
