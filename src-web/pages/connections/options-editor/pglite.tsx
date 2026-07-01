import { t } from '../../../i18n'
import { PGliteConfig } from '../../../tauri'
import { ConnectionEditorOptions } from '../connections'
import { DatabasePathSelect, InitialSQL, Item, Readonly } from '../from'
import { useOptions } from '../hooks'
import { ConnectionTab } from '../tabs'

export const PGliteConnection = ({ data, onChange }: ConnectionEditorOptions<PGliteConfig>) => {
    const { name, options, setName, setOpt } = useOptions(data, onChange)

    const general = (
        <>
            <Item label={t('name')} value={name} onChange={setName} />
            <DatabasePathSelect path={options.path} onChange={(path) => setOpt('path', path)} />
        </>
    )

    const security = (
        <Readonly secure={false} readonly={options.readonly} onChange={(val) => setOpt('readonly', val)} />
    )

    const initSQL = (
        <InitialSQL
            placeholder={`SET client_encoding TO 'UTF8';\nSET statement_timeout TO '10s';`}
            sql={options.initial}
            onChange={(val) => setOpt('initial', val)}
        />
    )

    return <ConnectionTab general={general} security={security} initialSQL={initSQL} alert='dev' />
}
