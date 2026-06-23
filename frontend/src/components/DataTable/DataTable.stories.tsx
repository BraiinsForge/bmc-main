import { action } from 'storybook/actions';
import type { ArgTypes } from '@storybook/react';
import * as get from '@/mocks';
import { useState } from 'react';

import {
    DataTable as Component,
    View as Pure,
    type DataTableProps,
    type DataTableRowBoolMap,
    type DataTableHeader,
    type DataTableRow,
} from './DataTable';
import { Datetime, Percentage, Button } from '@/components';
import { Edit as IconEdit, TrashCan as IconTrashcan } from '@carbon/react/icons';

type CellKey = '1' | '2' | '3' | '4' | '5' | '6' | '7' | '8' | '9';

const getCells = (): DataTableRow<CellKey>['cells'] => ({
    1: <Datetime value={Math.floor(Date.now() / 1e3)} />,
    2: <Percentage value={get.number(1.1, 3.3, true)} />,
    3: <Percentage value={get.number(1.1, 3.3, true)} />,
    4: <Percentage value={get.number(1.1, 3.3, true)} />,
    5: get.sentence(2, 5),
    6: get.sentence(2, 5),
    7: get.sentence(2, 5),
    8: get.sentence(2, 5),
    9: get.sentence(2, 5),
});
const getRow = (id: number, withExpansion: boolean): DataTableRow<CellKey> => ({
    id: String(id),
    cells: getCells(),
    expandedContent: withExpansion ? <div children={get.sentence(50)} /> : undefined,
});
const disable = Object.freeze({ table: { disable: true } });
const noop = () => {};
const colProps = (color: string) => ({ style: { color, whiteSpace: 'nowrap' } }) as const;

export default {
    title: 'components/DataTable',
    component: Pure,
};

interface Args extends Omit<DataTableProps, 'headers'> {
    $hasData: boolean;
    $rowsCount: number;
    $hasSelection: boolean;
    $hasExpansion: boolean;
    $complexHeaders: boolean;
    $appendFooter: boolean;
}
function Demo(args: Args) {
    const { $hasData, $rowsCount, $hasSelection, $hasExpansion, $complexHeaders, $appendFooter, ...$args } = args;
    const [selectionState, selectionSetState] = useState<DataTableRowBoolMap>(new Map());
    const [searchState, searchSetState] = useState<string>('');

    const bottomRowHeaders: Array<DataTableHeader<CellKey>> = [
        { colProps: colProps('#ff0000'), key: '1', header: $complexHeaders ? null : '### 1st ###' },
        { colProps: colProps('#ff7700'), key: '2', header: '### 2nd ###' },
        { colProps: colProps('#ffb700'), key: '3', header: '### 3rd ###' },
        { colProps: colProps('#c8ff00'), key: '4', header: '### 4th ###' },
        { colProps: colProps('#00ffa6'), key: '5', header: $complexHeaders ? null : '### 5th ###' },
        { colProps: colProps('#00d4ff'), key: '6', header: '### 6th ###' },
        { colProps: colProps('#0094ff'), key: '7', header: '### 7th ###' },
        { colProps: colProps('#002eff'), key: '8', header: '### 8th ###' },
        { colProps: colProps('#6a00ff'), key: '9', header: '### 9th ###' },
    ];
    const props: DataTableProps = {
        ...$args,
        headers: $complexHeaders
            ? [
                  [
                      { header: '1-1', rowSpan: 2 },
                      { header: '1-2', colSpan: 7, align: 'center' },
                      { header: '1-3', rowSpan: 2 },
                  ],
                  bottomRowHeaders,
              ]
            : bottomRowHeaders,
        rows: $hasData ? get.arrayOf($rowsCount, i => getRow(i, $hasExpansion && i % 2 === 0)) : [],
    };

    if ($appendFooter && $hasData) {
        props.rows.push(
            {
                id: 'footer-label',
                cells: [
                    {
                        children: 'Footer Label',
                        style: { fontWeight: 'bold', textTransform: 'uppercase' },
                        colSpan: 50,
                    },
                ],
            },
            {
                id: 'footer-content',
                cells: [
                    {
                        children: 'first',
                        colSpan: 3,
                        style: { textAlign: 'right' },
                    },
                    {
                        children: 'second',
                        colSpan: 10,
                        style: { textAlign: 'left' },
                    },
                ],
            },
        );
    }
    if ($hasSelection) {
        props.selection = {
            mode: 'multi',
            onSelect(v) {
                selectionSetState(v);
                action('onSelect')(v);
            },
            selectAllBehavior: 'page',
            selectedRows: selectionState,
            onCancel() {
                selectionSetState(new Map());
                action('onCancel')();
            },
            selectedRowsTotal: Object.values(selectionState).filter(Boolean).length,
        };
    } else {
        delete props.selection;
    }

    return (
        <Component
            {...props}
            key={JSON.stringify(args)}
            toolbar={{
                tableBatchActions: [
                    <Button id="edit" key="edit" icon={IconEdit} children="Edit" onClick={noop} />,
                    <Button
                        id="deleteSelected"
                        key="deleteSelected"
                        icon={IconTrashcan}
                        children="Delete selected"
                        onClick={noop}
                    />,
                ],
                tableToolbarContent: [
                    <Component.TableToolbarSearch
                        id="workers-filter"
                        key="workers-filter"
                        placeholder="Input string or RegExp to filter the table"
                        defaultValue={searchState}
                        onChange={e => searchSetState(typeof e === 'string' ? e : e.target.value)}
                    />,
                    <div
                        key="secondaryHashrate"
                        data-floating-menu-container
                        tabIndex={-1}
                        children={
                            <Component.TableToolbarMenu className="cds--label">
                                <Component.TableToolbarAction
                                    className="cds--label"
                                    children="Secondary HR"
                                    onClick={noop}
                                />
                            </Component.TableToolbarMenu>
                        }
                    />,
                    <Button onClick={noop} children="Labels" key="labels" id="labels" />,
                    <Button onClick={noop} children="Refresh" key="refresh" id="refresh" />,
                    <Button onClick={noop} children="Connect" key="connect" id="connect" />,
                ],
            }}
        />
    );
}

export const DataTable = (args: Args) => <Demo {...args} />;
DataTable.storyName = 'DataTable';
DataTable.args = {
    $hasData: true,
    $rowsCount: 15,
    $hasSelection: true,
    $hasExpansion: true,
    $complexHeaders: true,
    $appendFooter: true,

    isLoading: false,
    placeholder: { message: "There's nothing to see here!" },
    skeletonRowsCount: 5,
    withRowBorders: false,
} as Args;
DataTable.argTypes = {
    $hasData: { name: '$hasData', control: { type: 'boolean' } },
    $rowsCount: { name: '$rowsCount', control: { type: 'range', min: 0, max: 50 } },
    $hasSelection: { name: '$hasSelection', control: { type: 'boolean' } },
    $hasExpansion: { name: '$hasExpansion', control: { type: 'boolean' } },
    $complexHeaders: { name: '$complexHeaders', control: { type: 'boolean' } },
    $appendFooter: { name: '$appendFooter', control: { type: 'boolean' } },

    rows: disable,
    toolbar: disable,
    selection: disable,
    placeholder: disable,
    className: disable,
    style: disable,
    intl: disable,
    skeletonRowsCount: { type: { name: 'number' }, control: { type: 'range', min: 1, max: 10 } },
} as ArgTypes<Args>;
