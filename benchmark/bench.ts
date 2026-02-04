import { Bench } from 'tinybench'
import { ScreenCapture } from '../index.mjs'

const b = new Bench()

b.add('ScreenCapture: init', () => {
  new ScreenCapture(() => {})
})

await b.run()

console.table(b.table())
