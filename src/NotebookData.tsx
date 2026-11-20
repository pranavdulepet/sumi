import {useState} from 'react';
import {call,type Row} from './api';
export function NotebookData({root}:{root:string}){
  const [confirm,setConfirm]=useState(false),[text,write]=useState(''),[status,note]=useState(''),[working,busy]=useState(false),[metrics,show]=useState<Row|null>(null);
  async function run(work:()=>Promise<void>){busy(true);note('');try{await work();}catch(e){note(String(e));}finally{busy(false);}}
  return <details><summary>Your notebook</summary>
    <p className="small">{root}</p>
    <button disabled={working} onClick={()=>run(async()=>{const saved=await call('exportNotebook');if(saved)note('Saved to '+saved);})}>Export a readable copy</button>
    <button onClick={()=>run(async()=>show(await call('notebookMeasures')))}>Review local measures</button>
    {metrics && <div><table><thead><tr><th>Week starting</th><th>Days used</th><th>Replies</th><th>Clarifications¹</th><th>Rejected</th><th>Deep</th></tr></thead><tbody>{metrics.weeks.map((w:Row)=><tr key={w.week}><td>{w.week}</td><td>{w.days.length}</td><td>{w.turns}</td><td>{w.clarifications}</td><td>{w.rejections}</td><td>{w.deep}</td></tr>)}</tbody></table>
    <small>¹ Follow-ups beginning with a clarification such as “I meant.” Rejected replies count corrections and regenerations.</small>
    <p>{metrics.eligibleSkills ? metrics.reusedSkills+' of '+metrics.eligibleSkills+' active skills were reused in their first month.':'Skills need a month of history to measure reuse.'}</p>
    {metrics.dreamTrial?.cases && <small>Last memory trial · {metrics.dreamTrial.cases} held-out replies · {metrics.dreamTrial.baseline} points before, {metrics.dreamTrial.candidate} after.</small>}</div>}
    <button onClick={()=>setConfirm(!confirm)}>Remove this notebook</button>
    {confirm && <form onSubmit={e=>{e.preventDefault();void run(async()=>{await call('trashNotebook',{confirm:text});});}}>
    <p>The entire folder at {root}, including files you added, will move to the Mac’s Trash. Sumi will reopen with a blank notebook.</p>
    <label>Type “Move notebook to Trash” to confirm<input value={text} onChange={e=>write(e.target.value)}/></label><button disabled={working||text!=='Move notebook to Trash'}>Move notebook to Trash</button>
    <button type="button" onClick={()=>{write('');setConfirm(false);}}>Cancel</button></form>}
    {status && <p role="status">{status}</p>}
  </details>;
}
