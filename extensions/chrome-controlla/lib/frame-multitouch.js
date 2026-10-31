// Child-frame two-finger pinch with a full identity preflight.
//
// The frame must still have the paired loader, a non-opaque HTTP(S) origin
// chain, a live child session, and a resolved target before any trusted touch
// event is sent. Geometry is mapped in child CSS pixels and revalidated
// immediately before dispatch. A delivery that started and could not be
// released remains `unknown`, never verified.

export async function dispatchFrameMultiTouch(request, respond, ctx) {
  const {
    isAuthorized, nativeRoute, resolveNativeLease, activationEpoch, focusEpoch,
    pairedTabDocumentIds, routeGeneration, debuggerSessions, frameTreeRegistry,
    pointerEngine, sendCdpCommand, rootPointForFrame, waitForFrameSession, safeNativeError,
  } = ctx;
  const tabId = ctx.tabId;
      const p=request.params||{},frameId=p.frame_id,loaderId=p.loader_id,backendNodeId=p.backend_node_id;
      const cx=p.center_x,cy=p.center_y,startDistance=p.start_distance,endDistance=p.end_distance,timeoutMs=p.timeout_ms??3000;
      const expectedActivationEpoch=p.expected_activation_epoch,expectedFocusEpoch=p.expected_focus_epoch;
      if(typeof frameId!=="string"||!frameId||frameId.length>256||typeof loaderId!=="string"||!loaderId||loaderId.length>256||
          !Number.isSafeInteger(backendNodeId)||backendNodeId<=0||![cx,cy,startDistance,endDistance].every(value=>Number.isFinite(value)&&value>=0&&value<=10000)||
          startDistance<40||startDistance>600||endDistance<20||endDistance>800||endDistance/startDistance<0.5||endDistance/startDistance>2||
          !Number.isSafeInteger(timeoutMs)||timeoutMs<1000||timeoutMs>10000||
          (expectedActivationEpoch!==undefined&&!Number.isSafeInteger(expectedActivationEpoch))||(expectedFocusEpoch!==undefined&&!Number.isSafeInteger(expectedFocusEpoch)))
        throw new Error("frame multi-touch requires bounded frame identity, two-finger geometry, and timeout.");
      const root={tabId},deadline=performance.now()+timeoutMs,documentId=ctx.pairedTabDocumentIds.get(tabId),generation=ctx.routeGeneration;
      const currentInfo=async()=>{
        const tree=await ctx.sendCdpCommand(root,"Page.getFrameTree",{});
        const visit=(node,ancestors=[])=>node?.frame?.id===frameId?{frame:node.frame,chain:[...ancestors,node.frame]}:(node?.childFrames||[]).map(child=>visit(child,[...ancestors,node.frame])).find(Boolean);
        return visit(tree.frameTree);
      };
      const validChain=info=>!!info?.chain&&info.chain.length>=2&&info.chain.length<=9&&info.chain.every(frame=>frame?.id&&frame?.loaderId);
      const safeOrigins=info=>validChain(info)&&info.chain.every(frame=>{
        if(typeof frame.securityOrigin!=="string"||frame.securityOrigin==="null")return false;
        try { const u=new URL(frame.url); return ["http:","https:"].includes(u.protocol)&&u.origin===frame.securityOrigin; } catch { return false; }
      });
      const sameChain=(a,b)=>validChain(a)&&validChain(b)&&a.chain.length===b.chain.length&&a.chain.every((frame,index)=>frame.id===b.chain[index].id&&frame.loaderId===b.chain[index].loaderId);
      const identityStillOwned=async()=>{
        if(performance.now()>=deadline||!ctx.isAuthorized(tabId)||ctx.pairedTabDocumentIds.get(tabId)!==documentId||
            (ctx.nativeRoute&&ctx.resolveNativeLease(request.lease_id,tabId)?.generation!==generation)||
            (expectedActivationEpoch!==undefined&&ctx.activationEpoch!==expectedActivationEpoch)||(expectedFocusEpoch!==undefined&&ctx.focusEpoch!==expectedFocusEpoch))return false;
        return (await chrome.tabs.get(tabId)).active===true;
      };
      const touchPoints=(center,distance)=>[
        {id:1,x:center.x-distance/2,y:center.y},
        {id:2,x:center.x+distance/2,y:center.y},
      ];
      const mapPinch=async(viewport,distance)=>{
        const left=await ctx.rootPointForFrame(tabId,frameId,objectId,cx-distance/2,cy,{width:viewport[0],height:viewport[1]});
        const right=await ctx.rootPointForFrame(tabId,frameId,objectId,cx+distance/2,cy,{width:viewport[0],height:viewport[1]});
        if(![left.x,left.y,right.x,right.y].every(value=>Number.isFinite(value)&&value>=0&&value<=10000))throw new Error("Frame pinch points could not be mapped into the page viewport.");
        const center={x:(left.x+right.x)/2,y:(left.y+right.y)/2};
        const mappedDistance=Math.hypot(right.x-left.x,right.y-left.y);
        if(!(mappedDistance>=20&&mappedDistance<=1200))throw new Error("Mapped frame pinch distance is outside the trusted input bounds.");
        return {left,right,center,distance:mappedDistance,signature:JSON.stringify([left.owner_signature,right.owner_signature])};
      };
      let sessionId,child,objectId,touchStartAttempted=false,dispatchAcknowledged=false,releaseAcknowledged=false,receipt;
      try {
        await ctx.debuggerSessions.ensureAutoAttach(tabId);
        const initialInfo=await currentInfo();
        if(!safeOrigins(initialInfo)||initialInfo.frame.loaderId!==loaderId)throw new Error("Frame identity or non-opaque HTTP(S) origin could not be proven.");
        sessionId=ctx.frameTreeRegistry.sessionForFrame(tabId,frameId)||await ctx.waitForFrameSession(tabId,frameId,400);
        child=sessionId?{tabId,sessionId}:root;
        const ax=await ctx.sendCdpCommand(child,"Accessibility.getFullAXTree",sessionId?{}:{frameId});
        if(!ax.nodes?.some(node=>node.backendDOMNodeId===backendNodeId))throw new Error("Pinch target is absent from the selected frame accessibility tree.");
        const resolved=await ctx.sendCdpCommand(child,"DOM.resolveNode",{backendNodeId});
        objectId=resolved.object?.objectId;
        if(!objectId)throw new Error("Pinch target could not be resolved in its child frame.");
        const guard="function(cx,cy,expectedViewport){const e=this,d=e?.ownerDocument,w=d?.defaultView;if(!e||!d||!w||!e.isConnected)return {ok:false,reason:'target_stale'};const b=e.getBoundingClientRect(),vw=w.innerWidth,vh=w.innerHeight,viewport=JSON.stringify([vw,vh,w.devicePixelRatio||1]),style=w.getComputedStyle(e);if(expectedViewport&&expectedViewport!==viewport)return {ok:false,reason:'frame_viewport_changed'};if(b.width<=0||b.height<=0||style.display==='none'||style.visibility!=='visible'||style.pointerEvents==='none'||Number.parseFloat(style.opacity||'1')<=0)return {ok:false,reason:'target_not_interactable'};const inside=cx>=b.left&&cx<=b.right&&cy>=b.top&&cy<=b.bottom,hit=cx>=0&&cx<vw&&cy>=0&&cy<vh?d.elementFromPoint(cx,cy):null;if(!inside||!(hit===e||e.contains(hit)))return {ok:false,reason:'pinch_target_not_interactable'};return {ok:true,viewport,rect:{left:b.left,top:b.top,width:b.width,height:b.height}};}";
        const guardCall=async(expectedViewport=null)=>{
          const result=await ctx.sendCdpCommand(child,"Runtime.callFunctionOn",{objectId,functionDeclaration:guard,arguments:[{value:cx},{value:cy},{value:expectedViewport}],returnByValue:true});
          return result?.result?.value;
        };
        const before=await guardCall();
        if(before?.ok!==true)throw new Error(before?.reason||"Frame pinch target preflight refused.");
        const viewport=[JSON.parse(before.viewport)[0],JSON.parse(before.viewport)[1]];
        const initial=await mapPinch(viewport,startDistance);
        const initialEnd=await mapPinch(viewport,endDistance);
        if(initial.signature!==initialEnd.signature)throw new Error("Frame owner transform changed across pinch geometry.");
        const finalGuard=await guardCall(before.viewport),finalInfo=await currentInfo();
        if(finalGuard?.ok!==true||JSON.stringify(finalGuard.rect)!==JSON.stringify(before.rect)||!sameChain(initialInfo,finalInfo)||
            ctx.frameTreeRegistry.sessionForFrame(tabId,frameId)!==(sessionId||undefined))throw new Error("Frame target, loader, or session changed during pinch preflight.");
        const finalStart=await mapPinch(viewport,startDistance);
        const finalEnd=await mapPinch(viewport,endDistance);
        const dispatchInfo=await currentInfo();
        const moved=Math.hypot(finalStart.center.x-initial.center.x,finalStart.center.y-initial.center.y)>0.5;
        const ownerChanged=finalStart.signature!==initial.signature||finalEnd.signature!==initialEnd.signature;
        const chainChanged=!sameChain(initialInfo,dispatchInfo),originChanged=!safeOrigins(dispatchInfo),identityChanged=!(await identityStillOwned());
        if(moved||ownerChanged||chainChanged||originChanged||identityChanged)throw new Error(`Frame pinch transform or document identity changed before trusted touch input (moved=${moved}, owner=${ownerChanged}, chain=${chainChanged}, origin=${originChanged}, identity=${identityChanged}).`);
        touchStartAttempted=true;
        await ctx.pointerEngine.touchMulti(root,"touchStart",touchPoints(finalStart.center,finalStart.distance));
        await ctx.pointerEngine.touchMulti(root,"touchMove",touchPoints(finalEnd.center,finalEnd.distance));
        dispatchAcknowledged=true;
        if(!sameChain(initialInfo,await currentInfo())||ctx.frameTreeRegistry.sessionForFrame(tabId,frameId)!==(sessionId||undefined)||!(await identityStillOwned()))throw new Error("Frame identity changed after pinch dispatch.");
        receipt={status:"dispatched",gesture:"two_finger_pinch",dispatch_acknowledged:true,dispatch_may_have_occurred:true,frame_id:frameId,frame_loader_id:loaderId,
          coordinate_space:"child_frame_css_viewport",root_center:finalStart.center,root_start_distance:finalStart.distance,root_end_distance:finalEnd.distance,
          start_distance:startDistance,end_distance:endDistance,viewport:{width:viewport[0],height:viewport[1]}};
      } catch(error) {
        receipt={status:touchStartAttempted?"unknown":"not_dispatched",reason_code:touchStartAttempted?"frame_pinch_delivery_unknown":"frame_pinch_preflight_refused",
          dispatch_acknowledged:dispatchAcknowledged,dispatch_may_have_occurred:touchStartAttempted,reason:ctx.safeNativeError(String(error))};
      } finally {
        if(touchStartAttempted)releaseAcknowledged=await Promise.race([ctx.pointerEngine.touchEnd(root).then(()=>true).catch(()=>false),new Promise(resolve=>setTimeout(()=>resolve(false),1000))]);
        if(objectId&&child)await ctx.sendCdpCommand(child,"Runtime.releaseObject",{objectId}).catch(()=>{});
      }
      if(touchStartAttempted&&!releaseAcknowledged)receipt={status:"unknown",reason_code:"touch_release_unconfirmed",dispatch_acknowledged:dispatchAcknowledged,dispatch_may_have_occurred:true};
      respond({type:"result",id:request.id,result:receipt||{status:"unknown",reason_code:"frame_pinch_receipt_missing",dispatch_acknowledged:dispatchAcknowledged,dispatch_may_have_occurred:touchStartAttempted}});
}
