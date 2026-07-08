import { ScrollView } from '../../../ui'
import { Footer } from './footer'

export const AboutSettings = () => {
    return (
        <ScrollView className='grow' axis='y'>
            <div className='grid grid-cols-[minmax(100px,auto)_1fr] gap-x-5 gap-y-3 p-4 text-xs text-tertiary'>
                <Footer />
            </div>
        </ScrollView>
    )
}
