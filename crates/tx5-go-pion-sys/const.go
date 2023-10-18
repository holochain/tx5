package main

const (
	// Reporting an error from the lib.
	// - allowed contexts: Event, Response
	// - msg slot_a = utf8 error id ptr
	// - msg slot_b = utf8 error id len
	// - msg slot_c = utf8 error info ptr
	// - msg slot_d = utf8 error info len
	TyErr UintPtrT = 0xffff

	// A tracing message published by the lib.
	// - allowed contexts: Event
	// - msg slot_a: logging level
	// - msg slot_b: utf8 info ptr
	// - msg slot_c: utf8 info len
	TyOnTrace UintPtrT = 0xfffe

	// Init MUST be called EXACTLY once before a peer con is created.
	// - allowed contexts: Call, Response
	// - msg slot_a: config buffer id
	TyTx5Init UintPtrT = 0x7001

	// Request a go buffer be created / giving access to said buffer in resp.
	// - allowed contexts: Call, Response
	// - msg slot_a: buffer id
	TyBufferAlloc UintPtrT = 0x8001

	// Request an existing buffer be released. It will no longer be accessible.
	// - allowed contexts: Call, Response
	// - call slot_a: buffer id
	TyBufferFree UintPtrT = 0x8002

	// Request access to an existing buffer.
	// - allowed contexts: Call, Response
	// - call slot_a: buffer id
	// - msg slot_a: buffer id
	// - msg slot_b: buffer ptr
	// - msg slot_c: buffer len
	TyBufferAccess UintPtrT = 0x8003

	// Request additional space be reserved for appending to buffer.
	// - allowed contexts: Call, Response
	// - call slot_a: buffer id
	// - call slot_b: additional length
	TyBufferReserve UintPtrT = 0x8004

	// Request existing buffer be extended with provided additional bytes.
	// - allowed contexts: Call, Response
	// - call slot_a: buffer id
	// - call slot_b: additional bytes ptr
	// - call slot_c: additional bytes len
	TyBufferExtend UintPtrT = 0x8005

	// Request access to bytes at beginning of existing buffer, advancing the
	// read cursor.
	// - allowed contexts: Call, Response
	// - call slot_a: buffer id
	// - call slot_b: max read length
	// - msg slot_a: buffer ptr
	// - msg slot_b: buffer len
	TyBufferRead UintPtrT = 0x8006

	// Request a new peer connection be opened.
	// - allowed contexts: Call, Response
	// - call slot_a: config buffer id
	// - msg slot_a: peer_con id
	TyPeerConAlloc UintPtrT = 0x9001

	// Request an existing peer con be closed and released.
	// - allowed contexts: Call, Response
	// - call slot_a: peer_con id
	TyPeerConFree UintPtrT = 0x9002

	// Request an existing peer con create an offer.
	// - allowed contexts: Call, Response
	// - call slot_a: peer_con id
	// - call slot_b: config buffer id
	// - msg slot_a: offer buffer id
	TyPeerConCreateOffer UintPtrT = 0x9003

	// Request an existing peer con create an answer.
	// - allowed contexts: Call, Response
	// - call slot_a: peer_con id
	// - call slot_b: config buffer id
	// - msg slot_a: answer buffer id
	TyPeerConCreateAnswer UintPtrT = 0x9004

	// Request an existing peer con set local description.
	// - allowed contexts: Call, Response
	// - call slot_a: peer_con id
	// - call slot_b: desc buffer id
	TyPeerConSetLocalDesc UintPtrT = 0x9005

	// Request an existing peer con set rem description.
	// - allowed contexts: Call, Response
	// - call slot_a: peer_con id
	// - call slot_b: desc buffer id
	TyPeerConSetRemDesc UintPtrT = 0x9006

	// Request an existing peer con add ice candidate.
	// - allowed contexts: Call, Response
	// - call slot_a: peer_con id
	// - call slot_b: ice buffer id
	TyPeerConAddICECandidate UintPtrT = 0x9007

	// Request an existing peer con create a new data channel.
	// - allowed contexts: Call, Response
	// - call slot_a: peer_con id
	// - call slot_b: config buffer id
	TyPeerConCreateDataChan UintPtrT = 0x9008

	// Request an existing peer con be closed and released.
	// - allowed contexts: Call, Response
	// - call slot_a: peer_con id
	// - msg slot_a: stats buffer id
	TyPeerConStats UintPtrT = 0x9009

	// OnICECandidate event on an existing peer con.
	// - allowed contexts: Event
	// - msg slot_a: peer_con id
	// - msg slot_b: ice buffer id
	TyPeerConOnICECandidate UintPtrT = 0x9801

	// OnConStateChange event on an existing peer con.
	// - allowed contexts: Event
	// - msg slot_a: peer_con id
	// - msg slot_b: state id
	TyPeerConOnStateChange UintPtrT = 0x9802

	// OnDataChannel event on an existing peer con.
	// - allowed contexts: Event
	// - msg slot_a: peer_con id
	// - msg slot_b: data_chan id
	TyPeerConOnDataChannel UintPtrT = 0x9803

	// Request an existing data chan be closed and released.
	// - allowed contexts: Call, Response
	// - call slot_a: data_chan id
	TyDataChanFree UintPtrT = 0xa002

	// Request the ready state of an existing data channel..
	// - allowed contexts: Call, Response
	// - call slot_a: data_chan id
	// - msg slot_a: ready state
	TyDataChanReadyState UintPtrT = 0xa003

	// Request an existing data chan send data to the remote end.
	// - allowed contexts: Call, Response
	// - call slot_a: data_chan id
	// - call slot_b: buffer id
	// - msg slot_a: cur buffer amount
	TyDataChanSend UintPtrT = 0xa004

	// Request the label of an existing data channel.
	// - allowed contexts: Call, Response
	// - call slot_a: data_chan id
	// - msg slot_a: label buffer id
	TyDataChanLabel UintPtrT = 0xa005

	// Set the buffered amount low threshold on an existing data channel.
	// - allowed contexts: Call, Response
	// - call slot_a: data_chan id
	// - call slot_b: threshold
	// - msg slot_a: cur buffer amount
	TyDataChanSetBufferedAmountLowThreshold UintPtrT = 0xa006

	// Get the amount of send data currently buffered on an existing data channel.
	// - allowed contexts: Call, Response
	// - call slot_a: data_chan id
	// - msg slot_a: cur buffer amount
	TyDataChanBufferedAmount UintPtrT = 0xa007

	// OnClose event on an existing data chan.
	// - allowed contexts: Event
	// - msg slot_a: data_chan id
	TyDataChanOnClose UintPtrT = 0xa801

	// OnOpen event on an existing data chan.
	// - allowed contexts: Event
	// - msg slot_a: data_chan id
	TyDataChanOnOpen UintPtrT = 0xa802

	// OnError event on an existing data chan.
	// - allowed contexts: Event
	// - msg slot_a: data_chan id
	// - msg slot_b: data ptr
	// - msg slot_c: data len
	TyDataChanOnError UintPtrT = 0xa803

	// OnMessage event on an existing data chan.
	// - allowed contexts: Event
	// - msg slot_a: data_chan id
	// - msg slot_b: data ptr
	// - msg slot_c: data len
	TyDataChanOnMessage UintPtrT = 0xa804

	// OnBufferedAmountLow event on an existing data chan.
	// - allowed contexts: Event
	// - msg slot_a: data_chan id
	TyDataChanOnBufferedAmountLow UintPtrT = 0xa805
)
