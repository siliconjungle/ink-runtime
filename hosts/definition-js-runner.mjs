import {createInterface} from 'node:readline';
import {pathToFileURL} from 'node:url';
const {invoke}=await import(pathToFileURL(process.argv[2]).href);
for await(const line of createInterface({input:process.stdin,crlfDelay:Infinity})){let response;try{const message=JSON.parse(line);response={ok:invoke(message.function,message.args)};}catch(e){response={error:String(e?.message??e)};}process.stdout.write(JSON.stringify(response)+'\n');}
