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

import {
    Component,
    Fragment,
    createElement,
    type FunctionComponent,
    type TdHTMLAttributes,
    type HTMLAttributes,
    type MouseEvent,
    type ChangeEvent,
    type KeyboardEvent,
    type ComponentType,
} from 'react';
import { Key } from 'ts-key-enum';
import isEqual from 'react-fast-compare';
import { useIntl, type IntlShape } from 'react-intl';
import { isPlainObject, cloneDeep, debounce } from 'es-toolkit';

import { assertUnreachable } from '@/lib/ts';

// Components
import { ARIA } from '@/components/constants';
import {
    type CarbonIconType,
    Folder as IconFolder,
    InformationFilled as IconInformation,
    CaretSort as IconCaretSort,
    CaretSortUp as IconCaretSortUp,
    CaretSortDown as IconCaretSortDown,
} from '@carbon/react/icons';
import { Empty } from '@/components/Empty';
import { Checkbox } from '@/components/Checkbox';
import { Tooltip } from '@/components/Tooltip';
import { TableSelectRow, type TableSelectRowProps } from './TableSelectRow';
import {
    ButtonSkeleton,
    TableContainer,
    Table,
    TableHead,
    TableExpandHeader,
    TableBody,
    TableRow,
    TableCell,
    TableToolbar,
    TableToolbarAction,
    TableToolbarContent,
    TableToolbarSearch,
    TableToolbarMenu,
    TableBatchActions,
    type DataTableSize,
    TableExpandRow,
    TableExpandedRow,
} from '@carbon/react';

// Css
import cn from 'clsx';
import css from './DataTable.scss';

//
// Props
//

type Dict = Record<string, any>;
type RowIdSet = Set<DataTableRow['id']>;
type RowIdMap<T> = Map<DataTableRow['id'], T>;
export type DataTableRowBoolMap = RowIdMap<boolean>;
export type DataTableTdProps = TdHTMLAttributes<HTMLTableCellElement>;
function cloneSelection(current: RowIdSet | DataTableRowBoolMap): DataTableRowBoolMap {
    // Just a quick copy
    if (current instanceof Map) return new Map(current);

    // Convert to the more verbose structure
    if (current instanceof Set) return new Map(Array.from(current).map(x => [x, true]));

    return new Map();
}

type CommonHeaderProps = {
    // Layout
    colSpan?: number;
    rowSpan?: number;

    width?: StrNum;
    maxWidth?: StrNum;
    minWidth?: StrNum;

    align?: 'left' | 'start' | 'center' | 'right' | 'end';
    sticky?: 'left' | 'right';

    // Manual tweaks
    colProps?: Omit<DataTableTdProps, 'children'>;
    cellProps?: Omit<DataTableTdProps, 'children'>;
};
export type DataTableHeader<K extends PropertyKey = string> = CommonHeaderProps & {
    key: K;

    // `null | undefined | NaN` means that the cell won't be rendered
    header?: Maybe<ReactNode> | typeof NaN;
    tooltip?: ReactNode;

    onSort?(id: K): void;
    sortDirection?: 'desc' | 'asc' | null;
};
export type DataTableHeaderArtificial = CommonHeaderProps & { header: NonNullable<ReactNode> };
export type DataTableCell = ReactNode | DataTableTdProps;

export type DataTableRow<Cells extends PropertyKey = string> = {
    id: string | number | bigint;
    props?: HTMLAttributes<HTMLTableRowElement> & { 'data-cy'?: string };
    expandedContent?: null | ReactNode;
    selection?: {
        disabled?: boolean;
        checked?: boolean;
        tooltip?: ReactNode;
    };
    cells: Record<Cells, DataTableCell> | Array<DataTableCell>;
};

// Either a single row of headers
type HeadersSingleRow<K extends PropertyKey = string> = Array<DataTableHeader<K>>;
/**
 * Or an array of rows where the last one must contain at least everything
 * that carbon accepts as a table header exept for the `header` attribute.
 * Leaving out the `header` attribute means that the cell won't be rendered
 */
type HeadersMultiRow<K extends PropertyKey = string> = Array<Array<DataTableHeader<K> | DataTableHeaderArtificial>>;

export interface CustomCheckboxComponentProps extends TableSelectRowProps {
    rowID: DataTableRow['id'];
    UpstreamComponent: FunctionComponent<TableSelectRowProps>;
}

export type DataTableProps<ColumnID extends PropertyKey = string> = {
    // Data
    headers: HeadersSingleRow<ColumnID> | HeadersMultiRow<ColumnID>;
    rows: Array<DataTableRow<ColumnID>>;

    // State
    isLoading?: boolean;

    selection?: {
        mode: 'single' | 'multi';
        selectAllBehavior: 'page' | 'all';

        selectedRows: RowIdSet | DataTableRowBoolMap;
        selectedRowsTotal: number;

        onSelect(selectedRows: DataTableRowBoolMap): void;
        onCancel(): void;

        // If supplied, the selection status message will have a different wording
        // and will include a link to select all available entries (calling this handler).
        onSelectAll?(): void;
        selectAllTooltip?: ReactNode;
        totalItemsCount?: number;

        CheckboxComponent?: ComponentType<CustomCheckboxComponentProps>;
    };
    expandedRows?: DataTableRowBoolMap;
    onExpand?(expandedRows: DataTableRowBoolMap): void;

    // Toolbar
    toolbar?: {
        tableBatchActions?: ReactNode[];
        tableToolbarContent?: ReactNode[];
        children?: ReactNode;
        className?: string;
        style?: CSSProperties;
    };
    toolbar2?: ReactNode;

    // Placeholder
    placeholder?: {
        icon?: CarbonIconType;
        iconSize?: number;
        title?: ReactNode;
        message?: ReactNode;
        controls?: ReactNode;
    };
    skeletonRowsCount?: number;

    // Styling
    withRowBorders?: boolean;
    overflowVisible?: boolean;
    size?: DataTableSize;

    className?: string;
    style?: CSSProperties;

    // FIXME:
    //  Only here because the skeleton used here for the loading state is actually using THIS component
    //  and giving it it's own content… this means that we need to overwrite this value from the "skeleton"
    'aria-busy'?: boolean;
};
type Props<ColumnID extends PropertyKey = string> = DataTableProps<ColumnID> & { intl: IntlShape };

//
// State
//

type StateHeaders = {
    isMultiHeader: boolean;
    // Headers normalized to the multi-header format
    normalizedHeaders: HeadersMultiRow;
    // Carbon needs a single row well-defined headers
    controlHeaders: Array<DataTableHeader>;
};
type StateRows = {
    // Signals if any row in the table has expandable content.
    // If so, we'll act accordingly while rendering the rows.
    // This allows us to have the expandable content optional on all rows and pads then if needed.
    hasRowWithExpandedContent: boolean;
    expandedRows: DataTableRowBoolMap;
};
type StateMemoize = {
    prevHeaders: Props['headers'];
};
type State = StateHeaders & StateRows & StateMemoize;
const getInitialState = (): State => ({
    prevHeaders: [],
    controlHeaders: [],
    isMultiHeader: false,
    normalizedHeaders: [],

    expandedRows: new Map(),
    hasRowWithExpandedContent: false,
});

function processHeaders(headers: Props['headers']): StateHeaders {
    const isMultiHeader = headers.length > 0 && (headers as (any | Dict)[][]).every(Array.isArray.bind(Array));
    const normalizedHeaders = isMultiHeader ? (headers as HeadersMultiRow) : [headers as HeadersSingleRow];
    // The presumption is that the last row is reference one for all body cells
    const controlHeaders = normalizedHeaders.at(-1) as HeadersSingleRow;

    return { isMultiHeader, normalizedHeaders, controlHeaders };
}
function processRows(rows: Props['rows']): StateRows['hasRowWithExpandedContent'] {
    return !!rows.find(row => row.expandedContent);
}
function noop() {}

function isElement(
    element: HTMLElement,
    match: {
        tagName: string;
        className?: string;
        attributes?: Record<string, string>;
    },
): element is HTMLInputElement {
    if (element.tagName !== match.tagName.toUpperCase()) return false;
    if (match.className && !element.classList.contains(match.className)) return false;
    if (Object.entries(match.attributes || {}).some(([k, v]) => element.getAttribute(k) !== v)) return false;

    return true;
}
type CheckboxInfo = {
    rowIndex: number;
    wasChecked: boolean;
};

export class View<ColumnID extends string> extends Component<Props<ColumnID>, State> {
    static defaultProps = {
        withRowBorders: true,
    };
    readonly state = getInitialState();
    static getDerivedStateFromProps(props: Props, state: State) {
        const res: Partial<State> = {};

        const didChangeHeaders = !isEqual(props.headers, state.prevHeaders);
        if (didChangeHeaders) Object.assign(res, processHeaders(props.headers));
        res.hasRowWithExpandedContent = processRows(props.rows);

        return res;
    }

    #translateWithID = (
        id: 'carbon.table.batch.cancel' | 'carbon.table.batch.items.selected' | 'carbon.table.batch.item.selected',
        data?: { totalSelected: number },
    ): ReactNode => {
        const {
            selection,
            rows,
            intl: { formatMessage },
        } = this.props;

        // Desctructure relevant numbers
        const selected: number = data?.totalSelected ?? 0;
        const available: number = selection?.totalItemsCount ?? 0;
        const selectAll = selection?.onSelectAll;

        // `{n} item[s] selected`
        let selectedMessage: ReactNode = formatMessage(
            { defaultMessage: '{selected, plural, =1 {# item} other {# items}} selected' },
            { selected },
        );

        // `All {n} available item[s] selected`
        if (selected === available) {
            selectedMessage = formatMessage(
                { defaultMessage: 'All {selected, plural, =1 {# item} other {# items}} available items are selected' },
                { selected },
            );
        }

        // `All {m} items on this page are selected. Select all {n} items`
        else if (
            // multi-selection is active
            selection?.mode === 'multi' &&
            // "select everything" mode is not active
            // This message would be redundant since the extra functionality
            // is replacing the default one.
            selection.selectAllBehavior !== 'all' &&
            // the number of available items is known
            available > 0 &&
            // handler exists
            selectAll &&
            // everything on this page is selected
            rows.every(r => this.#isSelected(r.id) || !this.#isSelectable(r.id))
        ) {
            selectedMessage = formatMessage(
                {
                    defaultMessage:
                        'All {selected, plural, =1 {# item} other {# items}} on this page are selected. <a>Select all {available, plural, =1 {# item} other {# items}}</a>',
                },
                {
                    a: ch => <a children={ch} {...ARIA.button(selectAll)} className={css.headerSelectAllLink} />,
                    selected,
                    available,
                },
            );
        }

        switch (id) {
            case 'carbon.table.batch.items.selected':
            case 'carbon.table.batch.item.selected':
                return selectedMessage;

            case 'carbon.table.batch.cancel':
                return formatMessage({ defaultMessage: 'Cancel' });

            default:
                assertUnreachable(id);
        }
    };

    #isSelectable(id: DataTableRow['id']): boolean {
        const { selection } = this.props;
        if (!selection) return false;

        const row = this.props.rows.find(x => x.id === id);
        if (!row) return false;

        return !row.selection?.disabled;
    }
    #isSelected(id: DataTableRow['id']): boolean {
        const d = this.props.selection?.selectedRows;
        if (!d) return false;

        return d instanceof Set ? d.has(id) : !!d.get(id);
    }
    #handleSelect = (rowIDs: Array<DataTableRow['id']>, checked: boolean): void => {
        const { selection } = this.props;
        if (!selection) return;

        // Radio button mode
        if (selection.mode === 'single') {
            selection.onSelect(new Map([[rowIDs[0], checked]]));
            return;
        }

        const res: DataTableRowBoolMap = cloneSelection(selection.selectedRows);

        // Toggle state of selected item
        rowIDs.forEach(id => {
            res.set(id, checked);
        });

        selection.onSelect(res);
    };
    #handleSelectAllClick = (e: ChangeEvent<HTMLInputElement>): void => {
        const { selection, rows } = this.props;
        this.#lastCheckboxClickInfo = undefined;
        this.#lastCheckboxDragInfo = undefined;
        if (!selection) return;

        const res: DataTableRowBoolMap = cloneSelection(selection.selectedRows);

        // Toggle state of items on the current page
        rows.forEach(x => {
            res.set(x.id, e.target.checked);
        });

        if (selection.selectAllBehavior === 'all' && selection.onSelectAll) selection.onSelectAll();
        else selection.onSelect(res);
    };

    #getCheckboxInfo = (el: HTMLElement, interactionKind: 'drag' | 'click'): undefined | CheckboxInfo => {
        // Carbon does a fuggly thing with label element's click handler to prevent event doubling.
        // It causes a problem, though, because we now get different event targets depending on certain aspects
        // of the event (e.g., drag vs click & whether a shift key is pressed).
        const isLabel = isElement(el, { tagName: 'label', className: 'cds--checkbox-label' });
        const isInput = isElement(el, { tagName: 'input', className: 'cds--checkbox' });
        if (!isLabel && !isInput) return;

        const tr = el.closest<HTMLTableRowElement>('tr');
        if (!tr) return;

        // The "input" element is queried through the target's parent
        // because it can (but doesn't have to) be child or sibling of "label"
        const input = el.closest<HTMLTableCellElement>('td')?.querySelector<HTMLInputElement>('input');
        const wasChecked: boolean =
            interactionKind === 'click'
                ? // When the user did the whole "click" interaction, the checkbox is already in the new state => invert
                  !input?.checked
                : // Whereas, when the user is dragging, the checkbox is still in the old state => just cast to boolean
                  !!input?.checked;
        const rowIndex: number = Array.from((tr.parentElement as HTMLTableSectionElement).children).indexOf(tr);

        return { rowIndex, wasChecked };
    };

    #lastCheckboxDragInfo?: CheckboxInfo;
    #handleMouseDownCapture = (e: MouseEvent<HTMLDivElement>): void => {
        this.#lastCheckboxDragInfo = this.#getCheckboxInfo(e.target as HTMLElement, 'drag');
    };
    #handleMouseUpCapture = (e: MouseEvent<HTMLDivElement>): void => {
        const prev = this.#lastCheckboxDragInfo;
        const curr = this.#getCheckboxInfo(e.target as HTMLElement, 'drag');

        // Abort if we don't have data or if the checkbox is the same as the one from "mouseDown" (=> just a "click")
        if (!curr || !prev) return;
        if (curr.rowIndex === prev.rowIndex) return;

        const desiredState = !prev.wasChecked;
        const indexStart = Math.min(curr.rowIndex, prev.rowIndex);
        const indexEnd = Math.max(curr.rowIndex, prev.rowIndex);
        const rowIDs = this.props.rows.slice(indexStart, indexEnd + 1).map(x => x.id);
        this.#handleSelect(rowIDs, desiredState);
    };

    #lastCheckboxClickInfo?: CheckboxInfo;
    #$handleClickCapture = (event: MouseEvent<HTMLDivElement>): void => {
        const prev = this.#lastCheckboxClickInfo;
        const curr = this.#getCheckboxInfo(event.target as HTMLElement, 'click');

        // Abort if we don't have data
        // (=> not a checkbox click)
        if (!curr) {
            this.#lastCheckboxClickInfo = undefined;
            this.#lastCheckboxDragInfo = undefined;
            return;
        }

        // Remember the last clicked checkbox
        // so that we can use it for the range selection
        if (!event.shiftKey) this.#lastCheckboxClickInfo = cloneDeep(curr);

        // Abort if we don't have data about previous checkbox,
        // or if they are the same one (no selection has been made)
        if (!prev || curr.rowIndex === prev.rowIndex) return;

        // If the user holds "Shift" and clicks on a checkbox,
        // we want to select all items between the last and the current one
        if (event.shiftKey) {
            const indexStart = Math.min(curr.rowIndex, prev.rowIndex);
            const indexEnd = Math.max(curr.rowIndex, prev.rowIndex);

            const rowIDs = this.props.rows.slice(indexStart, indexEnd + 1).map(x => x.id);

            this.#handleSelect(rowIDs, !prev.wasChecked);

            // Clear the state when we handle the range selection
            this.#lastCheckboxClickInfo = undefined;
            this.#lastCheckboxDragInfo = undefined;
        }
    };
    #handleClickCapture = debounce(this.#$handleClickCapture, 100);

    #isExpanded(id: DataTableRow['id']): boolean {
        return !!(this.props.expandedRows ?? this.state.expandedRows).get(id);
    }
    #handleExpandToggle = (id: DataTableRow['id'], expanded: boolean): void => {
        const d = this.props.expandedRows ?? this.state.expandedRows;
        const expandedRows = new Map(d.entries());
        expandedRows.set(id, expanded);

        this.setState({ expandedRows });
        this.props.onExpand?.(expandedRows);
    };

    render() {
        const {
            rows,
            headers,
            selection,
            placeholder,
            size,
            overflowVisible,
            withRowBorders,
            toolbar,
            toolbar2,
            isLoading,
            skeletonRowsCount,
            // DOM props
            style,
            className,
            intl,
            ...restProps
        } = this.props;
        const { formatMessage } = intl;
        const { isMultiHeader, controlHeaders, hasRowWithExpandedContent } = this.state;

        // Header cells
        const headerRowsData: HeadersMultiRow = isMultiHeader
            ? (headers as HeadersMultiRow)
            : [headers as HeadersSingleRow];
        const headersCount = headerRowsData.length;

        // Prepend checkbox cell when selectable
        const lastHeaderLeadCells: ReactNode[] = [];
        const placehoderHeaderCells: ReactNode[] = [];
        const checkboxCellClassName = cn(css.checkboxCell, css.stickyCell, isRTL() ? css.right : css.left);

        let selectedRowsOnPageCount: number = 0;

        // Selection header
        if (selection) {
            const key = 'placeholder-cell-selection';
            let element: ReactNode;

            if (selection.mode === 'single') {
                element = <th key={key} className={checkboxCellClassName} />;
            } else {
                // This has to be a function component so that it has a correct "selectedRowsOnPageCount" value.
                // Just storing the component into a variable would result in it always having the value of `0`!
                const BoundCheckbox: FunctionComponent = () => {
                    return (
                        <Checkbox
                            id={key}
                            name={key}
                            disabled={isLoading || rows.length === 0}
                            checked={selectedRowsOnPageCount > 0 && selectedRowsOnPageCount === rows.length}
                            indeterminate={selectedRowsOnPageCount > 0 && selectedRowsOnPageCount < rows.length}
                            onChange={this.#handleSelectAllClick}
                        />
                    );
                };
                element = (
                    <th
                        key={key}
                        className={checkboxCellClassName}
                        children={
                            selection.selectAllTooltip ? (
                                <Tooltip
                                    placement="top-end"
                                    trigger="hover"
                                    content={selection.selectAllTooltip}
                                    render={setTriggerRef => <div ref={setTriggerRef} children={<BoundCheckbox />} />}
                                />
                            ) : (
                                <BoundCheckbox />
                            )
                        }
                    />
                );
            }

            lastHeaderLeadCells.push(element);
            placehoderHeaderCells.push(<th key={key} />);
        }

        // "Expansion" header
        if (hasRowWithExpandedContent) {
            const expandRow = formatMessage({ defaultMessage: 'Expand row' });
            lastHeaderLeadCells.push(
                <TableExpandHeader
                    key="expand-header"
                    enableToggle={false}
                    onExpand={noop}
                    onClick={noop}
                    aria-label={expandRow}
                />,
            );
            placehoderHeaderCells.push(<th key="placeholder-cell-expander" />);
        }

        // Keep a cache of inline styles computed for <th>s
        // so that we can re-use them for their <td>s as well
        const headersStyleCache: CSSProperties[] = [];
        const headerRows: Array<ReactNode[]> = headerRowsData.map((tr, trInd) => {
            const cells: ReactNode[] = tr.map(($header, i) => {
                // "Artificial" headers are not rendered
                if (!('header' in $header) || $header.header == null || Number.isNaN($header.header)) return null;

                const h = $header as DataTableHeader;
                const props = { ...h.colProps, ...h.cellProps };
                if (h.colSpan) props.colSpan = h.colSpan;
                if (h.rowSpan) props.rowSpan = h.rowSpan;

                props.style = { ...(props.style || {}) };
                headersStyleCache[i] = props.style;
                if (h.width != null) props.style.width = h.width;
                if (h.maxWidth != null) props.style.maxWidth = h.maxWidth;
                if (h.minWidth != null) props.style.minWidth = h.minWidth;

                const thPropsSortable: HTMLAttributes<HTMLTableCellElement> = {};
                const onSort = h.onSort;
                if (onSort) {
                    Object.assign(thPropsSortable, {
                        role: 'button',
                        tabIndex: 0,
                        className: css.sortableHeader,
                        onClick: () => {
                            onSort(h.key);

                            // Blur the active element on click so that we don't leave the focus border behind
                            // @ts-expect-error: Missing blur method in DOM api types
                            document.activeElement?.blur();
                        },
                        onKeyDown: (e: KeyboardEvent) => {
                            const k = e.key;
                            if (k !== Key.Enter && k !== ' ') return;

                            // [space] key scrolls the page by default,
                            e.preventDefault();
                            e.stopPropagation();

                            onSort(h.key);
                        },
                    });
                }

                const headerCellText: ReactNode = h.tooltip ? (
                    <Tooltip
                        key={h.key}
                        trigger="hover"
                        placement="top"
                        content={h.tooltip}
                        render={ref => (
                            <span ref={ref} className={css.headerTooltipTrigger}>
                                <span children={h.header} />
                                <IconInformation size={16} className={css.icon} />
                            </span>
                        )}
                    />
                ) : (
                    h.header
                );

                const headerCellContent = onSort ? (
                    <div key={h.key} className={cn(css.headerSortingWrapper, !!h.sortDirection && css.sortActive)}>
                        <span className={css.text} children={headerCellText} />
                        {h.sortDirection ? (
                            h.sortDirection === 'desc' ? (
                                <IconCaretSortUp />
                            ) : (
                                <IconCaretSortDown />
                            )
                        ) : (
                            <IconCaretSort />
                        )}
                    </div>
                ) : (
                    headerCellText
                );

                // The "TableHeader" component doesn't give us access
                // to everything we need for desired UX,
                // so we have got the direct way.
                return (
                    <th
                        {...props}
                        {...thPropsSortable}
                        key={i}
                        scope="col"
                        className={cn(
                            css.header,
                            h.align && css[h.align],
                            // the "sticky" prop is used to both trigger
                            // the sticky behavior and determine the side
                            h.sticky && [css.stickyCell, css[h.sticky]],
                            props.className,
                            thPropsSortable.className,
                        )}
                        children={headerCellContent}
                    />
                );
            });

            // Prepend special headers to last header row
            const isLastRow = trInd === headersCount - 1;
            cells.unshift(...(isLastRow ? lastHeaderLeadCells : placehoderHeaderCells));

            return cells;
        });

        // Body rows
        let tbodyRows: ReactNode[] = rows.map((row, rowInd) => {
            const isExpanded = this.#isExpanded(row.id);
            const isSelected = this.#isSelected(row.id);

            // Accounting for the batch selection checkbox state
            if (isSelected) selectedRowsOnPageCount += 1;

            // Body cells
            let resCells: ReactNode[] = [];
            const cells = row.cells;

            if (Array.isArray(cells)) {
                resCells = cells.map((cell, i) => {
                    const cellProps: DataTableTdProps = {};
                    if (isPlainObject(cell) && 'children' in (cell as Dict)) Object.assign(cellProps, cell);
                    else cellProps.children = cell as ReactNode;
                    return <TableCell key={i} {...cellProps} />;
                });
            } else {
                const $cells = cells as Dict;
                controlHeaders.forEach((header, cellIndex) => {
                    const key = header.key;
                    const cell = $cells[key];

                    // Props propagation from header
                    const cellProps: DataTableTdProps = { ...header.colProps };
                    if (isPlainObject(cell) && 'children' in (cell as Dict)) Object.assign(cellProps, cell);
                    else cellProps.children = cell;

                    const cellStyle: CSSProperties = {
                        ...headersStyleCache[cellIndex],
                        ...header.colProps?.style,
                    };
                    if (header.align != null) cellStyle.textAlign = header.align;

                    const cellClassName = cn(
                        cellProps.className,
                        // the "sticky" prop is used to both trigger
                        // the sticky behavior and determine the side
                        header.sticky && [css.stickyCell, css[header.sticky]],
                    );

                    resCells.push(<TableCell key={key} {...cellProps} style={cellStyle} className={cellClassName} />);
                });
            }

            // Prepend checkbox cell when selectable
            if (selection) {
                // Custom cells (array format) do not get a selection control
                if (Array.isArray(cells)) {
                    resCells.unshift(<TableCell key="selection-placeholder" children={'\xa0'} />);
                }

                // Data-driven cells get the chexbox / radio button
                else {
                    const key = 'TableSelectRow';
                    const id = `data-table-select-row-${row.id}`;
                    const props: TableSelectRowProps & { key: string } = {
                        key,
                        id,
                        name: id,
                        ariaLabel: formatMessage({ defaultMessage: 'Select row' }),
                        onChange: isChecked => this.#handleSelect([row.id], !!isChecked),
                        checked: row.selection?.checked ?? isSelected,
                        disabled: row.selection?.disabled,
                        radio: selection.mode === 'single',
                        className: checkboxCellClassName,
                        render(x) {
                            return row.selection?.tooltip ? (
                                <Tooltip
                                    trigger="hover"
                                    placement="bottom-end"
                                    content={row.selection?.tooltip}
                                    render={setTriggerRef => <div ref={setTriggerRef} children={x} />}
                                />
                            ) : (
                                x
                            );
                        },
                    };

                    resCells.unshift(
                        selection.CheckboxComponent
                            ? createElement(selection.CheckboxComponent, {
                                  ...props,
                                  rowID: row.id,
                                  UpstreamComponent: TableSelectRow,
                              })
                            : createElement(TableSelectRow, props),
                    );
                }
            }

            // Our added props for each row
            const rowProps = {
                // This being a TR, we don't want to spread TD attributes here
                ...(!('children' in row) ? row.props : {}),
                isExpanded,
                className: cn(isSelected && css.selectedRow, row.props?.className),
            };

            // Expandable rows are rendered differently
            if ('expandedContent' in row && row.expandedContent != null) {
                const key = `expandable-row-fragment-${row.id}`;
                return (
                    <Fragment key={key}>
                        <TableExpandRow
                            {...rowProps}
                            children={resCells}
                            onExpand={() => this.#handleExpandToggle(row.id, !isExpanded)}
                            // @ts-expect-error: Missing method in typing
                            onClick={() => {
                                // @ts-expect-error: Missing blur method in DOM api typing
                                document.activeElement?.blur();
                            }}
                        />

                        {isExpanded && (
                            <TableExpandedRow
                                colSpan={controlHeaders.length + 1 + Number(!!selection)}
                                children={row.expandedContent}
                                className={css.expandedRow}
                            />
                        )}
                    </Fragment>
                );
            }

            // If any row is expandable, but this one doesn't have the expanded content,
            // we'll need to add a "padding" cell so that all rows have correct cells count
            else if (hasRowWithExpandedContent) {
                resCells.unshift(<TableCell key={`expander-placeholder-${rowInd}`} className="cds--table-expand" />);
            }

            // Render the normal (unexpandable) row
            return <TableRow {...(rowProps as Dict)} key={row.id.toString()} children={resCells} />;
        });

        // Render Empty state placeholder
        let emptyStatePlaceholder: ReactNode = null;
        if (!isLoading && !tbodyRows.length) {
            /**
             * The idea behind rendering hidden rows/cells is that the size of the table
             * will respect the `skeletonRowsCount` prop which means that the size
             * should be the same between loading & rendered states.
             *
             * This is done by:
             *  - setting the tbody position to relative
             *  - rendering the placeholder in an additional row/cell at the end
             *  - stretching given placeholder row over the others
             */
            const count = skeletonRowsCount || 10;
            tbodyRows = new Array(count).fill(null).map((_, i) => {
                return (
                    <tr
                        key={i}
                        children={
                            <td children={'\xa0'} colSpan={1e3} style={{ border: 'none', pointerEvents: 'none' }} />
                        }
                    />
                );
            });
            emptyStatePlaceholder = (
                <Empty
                    icon={placeholder?.icon || IconFolder}
                    iconSize={placeholder?.iconSize ?? 48}
                    standaloneIcon
                    title={placeholder?.title}
                    message={placeholder?.message}
                    controls={placeholder?.controls}
                    className={css.placeholder}
                />
            );
        }

        // Glue it all together
        const rootStyle: CSSProperties = { ...style };
        if (emptyStatePlaceholder) rootStyle.overflow = 'hidden';

        // Render skeleton version if instructed to
        return isLoading ? (
            <DataTableSkeleton
                showToolbar={!!toolbar?.tableToolbarContent}
                rowCount={skeletonRowsCount || 10}
                headers={headers as HeadersSingleRow | HeadersMultiRow}
                controlHeaders={controlHeaders}
                size={size}
            />
        ) : (
            <TableContainer
                className={cn(
                    css.root,
                    emptyStatePlaceholder && css.hasPlaceholder,
                    overflowVisible && css.overflowVisible,
                    withRowBorders && css.withRowBorders,
                    isMultiHeader && css.isMultiHeader,
                    className,
                )}
                style={rootStyle}
                onMouseDownCapture={this.#handleMouseDownCapture}
                onMouseUpCapture={this.#handleMouseUpCapture}
                onClickCapture={this.#handleClickCapture}
            >
                {toolbar && (
                    <div
                        // TableToolbar doesn't accept classNames,
                        // so we need to wrap it in extra div
                        className={cn(css.toolbar, toolbar.className)}
                    >
                        <TableToolbar style={toolbar.style}>
                            {toolbar.children != null ? (
                                <div className={css.toolbarChildren} children={toolbar.children} />
                            ) : null}
                            {toolbar.tableToolbarContent && (
                                <TableToolbarContent children={toolbar.tableToolbarContent} />
                            )}
                            {selection && toolbar.tableBatchActions && (
                                <TableBatchActions
                                    onCancel={selection.onCancel}
                                    shouldShowBatchActions={selection.selectedRowsTotal > 0}
                                    totalSelected={selection.selectedRowsTotal}
                                    // @ts-expect-error: Invalid type-def in the typing module, returning ReactNode does work here!
                                    translateWithId={this.#translateWithID}
                                    children={toolbar.tableBatchActions}
                                />
                            )}
                        </TableToolbar>
                    </div>
                )}
                {toolbar2 ? <div className={css.toolbar2} children={toolbar2} /> : null}

                <Table
                    size={size}
                    isSortable={false}
                    aria-live="polite"
                    aria-busy={restProps['aria-busy'] ?? false}
                    tabIndex={-1}
                >
                    <TableHead
                        children={headerRows.map((x, i) => {
                            return <TableRow children={x} key={i} />;
                        })}
                    />

                    <TableBody style={{ position: 'relative' }}>
                        {emptyStatePlaceholder && (
                            <tr className={css.placeholderWrapper}>
                                <td className={css.placeholderWrapper} colSpan={1e3} children={emptyStatePlaceholder} />
                            </tr>
                        )}
                        {tbodyRows}
                    </TableBody>
                </Table>
            </TableContainer>
        );
    }
}

const isRTL = (): boolean => document.documentElement.getAttribute('dir') === 'rtl';

// react-intl v7 dropped the `injectIntl` HOC; inject `intl` via a hook wrapper.
// A generic function component keeps the per-`ColumnID` call signature, with the
// toolbar sub-components attached as statics.
function DataTable<ColumnID extends PropertyKey = string>(props: DataTableProps<ColumnID>): ReactElement {
    const intl = useIntl();
    return <View {...(props as DataTableProps<string>)} intl={intl} />;
}
DataTable.TableToolbarSearch = TableToolbarSearch;
DataTable.TableToolbarMenu = TableToolbarMenu;
DataTable.TableToolbarAction = TableToolbarAction;
export { DataTable };

export interface DataTableSkeletonProps {
    headers: DataTableProps['headers'];
    rowCount: number;
    size: DataTableProps['size'];
    showToolbar?: boolean;
    controlHeaders: DataTableHeader[];
}
export class DataTableSkeleton extends Component<DataTableSkeletonProps> {
    private getRows = (): DataTableRow[] => {
        const { rowCount, controlHeaders } = this.props;
        return new Array(rowCount).fill(null).map((_, i) => ({
            id: i,
            cells: new Array(controlHeaders.length).fill(<span className="cds--skeleton__text" />),
        }));
    };

    render() {
        const { headers, size, showToolbar } = this.props;
        return (
            <DataTable
                className={css.root}
                toolbar={showToolbar ? { tableToolbarContent: [<ButtonSkeleton key="button-skeleton" />] } : undefined}
                size={size}
                rows={this.getRows()}
                headers={headers}
                aria-busy
            />
        );
    }
}
